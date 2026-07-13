//! TLS-terminating reverse proxy: ACME certs via tls-common, Host-routed proxy to
//! cleartext upstreams, accept sharded across SO_REUSEPORT sockets.

mod config;
mod listen;
mod proxy;
mod routes;

use std::sync::Arc;
use std::time::Duration;

use hyper::body::Incoming;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::proxy::ProxyState;

fn main() {
    // rustls provider must be installed before any TLS use.
    tls_common::init_crypto();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gateway: config error: {e}");
            std::process::exit(2);
        }
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cfg.worker_threads)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("gateway: runtime error: {e}");
            std::process::exit(1);
        }
    };

    rt.block_on(async move {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(cfg.connect_timeout));
        connector.set_nodelay(true);
        connector.set_keepalive(Some(Duration::from_secs(60)));
        connector.enforce_http(true);

        let client: Client<HttpConnector, Incoming> = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(cfg.pool_idle_timeout)
            .pool_max_idle_per_host(cfg.pool_max_idle_per_host)
            .pool_timer(TokioTimer::new())
            .build(connector);

        let acme = if cfg.https_enabled {
            if let Err(e) = tokio::fs::create_dir_all(&cfg.acme_cache_dir).await {
                tracing::warn!("acme cache dir {:?}: {}", cfg.acme_cache_dir, e);
            }
            Some(tls_common::acme_acceptor(tls_common::AcmeParams {
                domains: cfg.acme_domains(),
                email: cfg.acme_email.clone(),
                cache_dir: cfg.acme_cache_dir.clone(),
                staging: cfg.acme_staging,
                alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            }))
        } else {
            None
        };

        let state = Arc::new(ProxyState {
            client,
            routes: cfg.routes.clone(),
            https_port: cfg.https_port,
        });

        if let Err(e) = listen::run(cfg, state, acme).await {
            eprintln!("gateway: {e}");
            std::process::exit(1);
        }
    });
}
