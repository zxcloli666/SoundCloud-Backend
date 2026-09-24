mod acceptor;
mod acme;
mod config;
mod proxy;
mod redirect;
mod serve;
mod shutdown;

pub use acme::{acme_acceptor, AcmeAcceptor, AcmeParams, TlsStream};
pub use config::{ProxyProtocolConfig, TlsConfig};
pub use serve::{serve, serve_http};
pub use shutdown::shutdown_signal;

pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
