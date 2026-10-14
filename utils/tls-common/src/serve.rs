use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use rustls::ServerConfig;
use rustls_acme::caches::DirCache;
use rustls_acme::AcmeConfig;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

use crate::acceptor::{ConnectInfoAcceptor, TrustedProxies};
use crate::config::{Http2Ping, ProxyProtocolConfig, TlsConfig};
use crate::redirect::redirect_router;
use crate::shutdown::shutdown_signal;

pub async fn serve(cfg: TlsConfig, app: Router) {
    crate::init_crypto();

    if let Err(e) = tokio::fs::create_dir_all(&cfg.cache_dir).await {
        warn!("failed to create ACME cache dir {:?}: {}", cfg.cache_dir, e);
    }

    let mut state = AcmeConfig::new(cfg.domains.clone())
        .contact_push(format!("mailto:{}", cfg.email))
        .cache(DirCache::new(cfg.cache_dir.clone()))
        .directory_lets_encrypt(!cfg.staging)
        .state();

    let mut rustls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(state.resolver());
    rustls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let trusted = trusted_proxies(&cfg.proxy).await;
    let https_acceptor = ConnectInfoAcceptor {
        inner: state.axum_acceptor(Arc::new(rustls_config)),
        proxy_protocol: cfg.proxy.enabled,
        proxy_trusted: trusted.clone(),
    };
    let http_acceptor = ConnectInfoAcceptor {
        inner: axum_server::accept::DefaultAcceptor::new(),
        proxy_protocol: cfg.proxy.enabled,
        proxy_trusted: trusted,
    };

    tokio::spawn(async move {
        while let Some(res) = state.next().await {
            match res {
                Ok(ok) => info!("acme event: {:?}", ok),
                Err(err) => error!("acme error: {:?}", err),
            }
        }
    });

    let https_addr = SocketAddr::from(([0, 0, 0, 0], cfg.https_port));
    let http_addr = SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
    let ping = Http2Ping::from_env();

    info!(
        "TLS: {} domain(s), https=:{} http=:{} redirect={} staging={} proxy_protocol={} h2_ping={:?}",
        cfg.domains.len(),
        cfg.https_port,
        cfg.http_port,
        cfg.http_redirect,
        cfg.staging,
        cfg.proxy.enabled,
        ping,
    );

    let http_app = if cfg.http_redirect {
        redirect_router(cfg.https_port)
    } else {
        app.clone()
    };

    let http_handle = axum_server::Handle::new();
    let https_handle = axum_server::Handle::new();

    let shutdown_handles = (http_handle.clone(), https_handle.clone());
    tokio::spawn(async move {
        shutdown_signal().await;
        let grace = Duration::from_secs(3);
        shutdown_handles.0.graceful_shutdown(Some(grace));
        shutdown_handles.1.graceful_shutdown(Some(grace));
    });

    let http_port = cfg.http_port;
    let http_task = tokio::spawn(async move {
        let mut server = axum_server::bind(http_addr)
            .handle(http_handle)
            .acceptor(http_acceptor);
        ping_idle_connections(server.http_builder(), ping);
        if let Err(e) = server.serve(http_app.into_make_service()).await {
            error!("HTTP :{} server error: {}", http_port, e);
        }
    });

    let https_port = cfg.https_port;
    let https_task = tokio::spawn(async move {
        let mut server = axum_server::bind(https_addr)
            .handle(https_handle)
            .acceptor(https_acceptor);
        ping_idle_connections(server.http_builder(), ping);
        if let Err(e) = server.serve(app.into_make_service()).await {
            error!("HTTPS :{} server error: {}", https_port, e);
        }
    });

    let _ = tokio::join!(http_task, https_task);
}

pub async fn serve_http(
    addr: SocketAddr,
    proxy: ProxyProtocolConfig,
    app: Router,
) -> io::Result<()> {
    let trusted = trusted_proxies(&proxy).await;
    let acceptor = ConnectInfoAcceptor {
        inner: axum_server::accept::DefaultAcceptor::new(),
        proxy_protocol: proxy.enabled,
        proxy_trusted: trusted,
    };
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(3)));
    });
    let mut server = axum_server::bind(addr).handle(handle).acceptor(acceptor);
    ping_idle_connections(server.http_builder(), Http2Ping::from_env());
    server.serve(app.into_make_service()).await
}

fn ping_idle_connections(builder: &mut Builder<TokioExecutor>, ping: Http2Ping) {
    if let Some(interval) = ping.interval {
        builder
            .http2()
            .timer(TokioTimer::new())
            .keep_alive_interval(interval)
            .keep_alive_timeout(ping.timeout);
    }
}

async fn trusted_proxies(config: &ProxyProtocolConfig) -> TrustedProxies {
    let mut resolved_by_host = resolve_hosts(&config.trusted_hosts).await;
    let initial = flattened_addresses(&resolved_by_host);
    if config.enabled && config.trusted_cidrs.is_empty() && initial.is_empty() {
        panic!("none of TLS_PROXY_TRUSTED_HOSTS resolved at startup");
    }
    let trusted = TrustedProxies {
        cidrs: Arc::new(config.trusted_cidrs.clone()),
        resolved: Arc::new(std::sync::RwLock::new(initial)),
    };
    if !config.trusted_hosts.is_empty() {
        let hosts = config.trusted_hosts.clone();
        let resolved = trusted.resolved.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                for host in &hosts {
                    if let Some(addresses) = resolve_host(host).await {
                        resolved_by_host.insert(host.clone(), addresses);
                    }
                }
                let addresses = flattened_addresses(&resolved_by_host);
                if !addresses.is_empty() {
                    if let Ok(mut current) = resolved.write() {
                        *current = addresses;
                    }
                }
            }
        });
    }
    trusted
}

async fn resolve_hosts(hosts: &[String]) -> HashMap<String, Vec<IpAddr>> {
    let mut resolved = HashMap::new();
    for host in hosts {
        if let Some(addresses) = resolve_host(host).await {
            resolved.insert(host.clone(), addresses);
        }
    }
    resolved
}

async fn resolve_host(host: &str) -> Option<Vec<IpAddr>> {
    let lookup = tokio::net::lookup_host(format!("{host}:0"));
    match tokio::time::timeout(Duration::from_secs(2), lookup).await {
        Ok(Ok(addresses)) => {
            let mut addresses = addresses.map(|address| address.ip()).collect::<Vec<_>>();
            addresses.sort_unstable();
            addresses.dedup();
            (!addresses.is_empty()).then_some(addresses)
        }
        Ok(Err(error)) => {
            warn!(host, %error, "trusted proxy hostname resolution failed");
            None
        }
        Err(_) => {
            warn!(host, "trusted proxy hostname resolution timed out");
            None
        }
    }
}

fn flattened_addresses(resolved: &HashMap<String, Vec<IpAddr>>) -> Vec<IpAddr> {
    let mut addresses = resolved.values().flatten().copied().collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    addresses
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hyper_util::rt::TokioIo;
    use hyper_util::service::TowerToHyperService;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    use super::*;

    const H2_PREFACE_AND_EMPTY_SETTINGS: &[u8] =
        b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00";

    async fn serve_connection(
        io: DuplexStream,
        ping: Http2Ping,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut builder = Builder::new(TokioExecutor::new());
        ping_idle_connections(&mut builder, ping);
        builder
            .serve_connection(TokioIo::new(io), TowerToHyperService::new(Router::new()))
            .await
    }

    #[test]
    fn zero_interval_turns_h2_ping_off() {
        assert_eq!(Http2Ping::from_secs(0, 20).interval, None);
        assert_eq!(
            Http2Ping::from_secs(30, 0),
            Http2Ping {
                interval: Some(Duration::from_secs(30)),
                timeout: Duration::from_secs(1),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silent_h2_peer_is_dropped_after_an_unanswered_ping() {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let serving = tokio::spawn(serve_connection(server, Http2Ping::from_secs(30, 20)));
        let started = tokio::time::Instant::now();

        client
            .write_all(H2_PREFACE_AND_EMPTY_SETTINGS)
            .await
            .unwrap();
        let mut received = Vec::new();
        tokio::time::timeout(Duration::from_secs(120), client.read_to_end(&mut received))
            .await
            .expect("server kept a silent h2 peer")
            .unwrap();

        assert_eq!(started.elapsed(), Duration::from_secs(50));
        assert!(serving.await.unwrap().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn h2_peer_that_answers_pings_stays_connected() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        tokio::spawn(serve_connection(server, Http2Ping::from_secs(30, 20)));
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(client))
                .await
                .unwrap();
        tokio::spawn(connection);

        tokio::time::sleep(Duration::from_secs(600)).await;
        let response = sender
            .send_request(
                Request::get("http://localhost/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn failed_refresh_can_retain_last_known_addresses() {
        let mut resolved = HashMap::from([
            ("first".to_owned(), vec!["192.0.2.1".parse().unwrap()]),
            ("second".to_owned(), vec!["192.0.2.2".parse().unwrap()]),
        ]);
        resolved.insert("first".to_owned(), vec!["192.0.2.3".parse().unwrap()]);

        assert_eq!(
            flattened_addresses(&resolved),
            vec![
                "192.0.2.2".parse::<IpAddr>().unwrap(),
                "192.0.2.3".parse::<IpAddr>().unwrap(),
            ]
        );
    }
}
