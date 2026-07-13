use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use socket2::{Domain, Protocol, Socket, Type};
use tls_common::AcmeAcceptor;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{debug, error, info};

use crate::config::{Config, HttpMode};
use crate::proxy::{self, ProxyState};

pub async fn run(
    cfg: Config,
    state: Arc<ProxyState>,
    acme: Option<AcmeAcceptor>,
) -> Result<(), String> {
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        tls_common::shutdown_signal().await;
        let _ = tx.send(true);
    });

    if cfg.http_enabled {
        let addr = SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
        let mut bound = 0;
        for _ in 0..cfg.shards {
            match reuseport_listener(addr) {
                Ok(l) => {
                    tokio::spawn(accept_http(l, state.clone(), rx.clone(), cfg.http_mode));
                    bound += 1;
                }
                Err(e) => error!("bind http {addr}: {e}"),
            }
        }
        if bound == 0 {
            return Err(format!("could not bind any HTTP listener on {addr}"));
        }
        info!(
            "HTTP :{} up — {}/{} shards, mode={}",
            cfg.http_port,
            bound,
            cfg.shards,
            if cfg.http_mode == HttpMode::Proxy { "proxy" } else { "redirect" }
        );
    }

    if cfg.https_enabled {
        let acme = acme.ok_or("HTTPS enabled but ACME acceptor missing")?;
        let addr = SocketAddr::from(([0, 0, 0, 0], cfg.https_port));
        let mut bound = 0;
        for _ in 0..cfg.shards {
            match reuseport_listener(addr) {
                Ok(l) => {
                    tokio::spawn(accept_https(l, state.clone(), rx.clone(), acme.clone()));
                    bound += 1;
                }
                Err(e) => error!("bind https {addr}: {e}"),
            }
        }
        if bound == 0 {
            return Err(format!("could not bind any HTTPS listener on {addr}"));
        }
        info!(
            "HTTPS :{} up — {}/{} shards, {} ACME domain(s), staging={}",
            cfg.https_port,
            bound,
            cfg.shards,
            cfg.acme_domains().len(),
            cfg.acme_staging
        );
    }

    let mut rx = rx;
    let _ = rx.wait_for(|stop| *stop).await;
    info!("shutdown — draining in-flight for up to {:?}", cfg.shutdown_grace);
    tokio::time::sleep(cfg.shutdown_grace).await;
    Ok(())
}

async fn accept_http(
    listener: TcpListener,
    state: Arc<ProxyState>,
    mut rx: watch::Receiver<bool>,
    mode: HttpMode,
) {
    loop {
        tokio::select! {
            _ = rx.changed() => break,
            res = listener.accept() => match res {
                Ok((tcp, peer)) => {
                    let _ = tcp.set_nodelay(true);
                    tokio::spawn(serve_http(tcp, peer, state.clone(), mode));
                }
                Err(e) => debug!("http accept: {e}"),
            },
        }
    }
}

async fn accept_https(
    listener: TcpListener,
    state: Arc<ProxyState>,
    mut rx: watch::Receiver<bool>,
    acme: AcmeAcceptor,
) {
    loop {
        tokio::select! {
            _ = rx.changed() => break,
            res = listener.accept() => match res {
                Ok((tcp, peer)) => {
                    let _ = tcp.set_nodelay(true);
                    tokio::spawn(serve_https(tcp, peer, state.clone(), acme.clone()));
                }
                Err(e) => debug!("https accept: {e}"),
            },
        }
    }
}

async fn serve_http(tcp: TcpStream, peer: SocketAddr, state: Arc<ProxyState>, mode: HttpMode) {
    let ip = peer.ip();
    let https_port = state.https_port;
    let svc = service_fn(move |req| {
        let state = state.clone();
        async move {
            let resp = match mode {
                HttpMode::Redirect => proxy::redirect_to_https(&req, https_port),
                HttpMode::Proxy => proxy::handle(state, req, ip, false).await,
            };
            Ok::<_, Infallible>(resp)
        }
    });
    if let Err(e) = auto::Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(TokioIo::new(tcp), svc)
        .await
    {
        debug!("http conn {peer}: {e}");
    }
}

async fn serve_https(tcp: TcpStream, peer: SocketAddr, state: Arc<ProxyState>, acme: AcmeAcceptor) {
    let tls = match acme.accept(tcp).await {
        Ok(Some(stream)) => stream,
        Ok(None) => return, // ACME TLS-ALPN-01 challenge, already answered
        Err(e) => {
            debug!("tls handshake {peer}: {e}");
            return;
        }
    };
    let ip = peer.ip();
    let svc = service_fn(move |req| {
        let state = state.clone();
        async move { Ok::<_, Infallible>(proxy::handle(state, req, ip, true).await) }
    });
    if let Err(e) = auto::Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(TokioIo::new(tls), svc)
        .await
    {
        debug!("https conn {peer}: {e}");
    }
}

/// One SO_REUSEPORT socket per shard — the kernel spreads accepts, avoiding
/// single-listener contention.
fn reuseport_listener(addr: SocketAddr) -> io::Result<TcpListener> {
    let domain = if addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
    let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    sock.set_reuse_address(true)?;
    sock.set_reuse_port(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&addr.into())?;
    sock.listen(4096)?;
    TcpListener::from_std(std::net::TcpListener::from(sock))
}
