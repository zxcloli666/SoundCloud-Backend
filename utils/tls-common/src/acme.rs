use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls_acme::caches::DirCache;
use rustls_acme::futures_rustls::rustls::server::Acceptor;
use rustls_acme::futures_rustls::server::TlsStream as FuturesTlsStream;
use rustls_acme::futures_rustls::LazyConfigAcceptor;
use rustls_acme::{is_tls_alpn_challenge, AcmeConfig};
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tracing::{error, info};

pub type TlsStream = Compat<FuturesTlsStream<Compat<TcpStream>>>;

pub struct AcmeParams {
    pub domains: Vec<String>,
    pub email: String,
    pub cache_dir: PathBuf,
    pub staging: bool,
    pub alpn: Vec<Vec<u8>>,
}

#[derive(Clone)]
pub struct AcmeAcceptor {
    serve: Arc<ServerConfig>,
    challenge: Arc<ServerConfig>,
}

pub fn acme_acceptor(p: AcmeParams) -> AcmeAcceptor {
    let mut state = AcmeConfig::new(p.domains)
        .contact_push(format!("mailto:{}", p.email))
        .cache(DirCache::new(p.cache_dir))
        .directory_lets_encrypt(!p.staging)
        .state();

    let mut serve = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(state.resolver());
    serve.alpn_protocols = p.alpn;
    let challenge = state.challenge_rustls_config();

    tokio::spawn(async move {
        loop {
            match state.next().await {
                Some(Ok(ok)) => info!("acme event: {:?}", ok),
                Some(Err(err)) => error!("acme error: {:?}", err),
                None => break,
            }
        }
    });

    AcmeAcceptor {
        serve: Arc::new(serve),
        challenge,
    }
}

impl AcmeAcceptor {
    pub async fn accept(&self, tcp: TcpStream) -> io::Result<Option<TlsStream>> {
        let handshake = LazyConfigAcceptor::new(Acceptor::default(), tcp.compat()).await?;
        if is_tls_alpn_challenge(&handshake.client_hello()) {
            let _ = handshake.into_stream(self.challenge.clone()).await?;
            return Ok(None);
        }
        let tls = handshake.into_stream(self.serve.clone()).await?;
        Ok(Some(tls.compat()))
    }
}
