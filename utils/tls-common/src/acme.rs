use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls_acme::acme::ACME_TLS_ALPN_NAME;
use rustls_acme::caches::DirCache;
use rustls_acme::futures_rustls::rustls::server::Acceptor;
use rustls_acme::futures_rustls::server::TlsStream as FuturesTlsStream;
use rustls_acme::futures_rustls::LazyConfigAcceptor;
use rustls_acme::{is_tls_alpn_challenge, AcmeConfig, ResolvesServerCertAcme};
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tracing::{error, info};

pub type TlsStream = Compat<FuturesTlsStream<Compat<TcpStream>>>;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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
    certificates_acceptor(vec![p.domains], &p.email, p.cache_dir, p.staging, p.alpn)
}

pub(crate) fn certificates_acceptor(
    certificates: Vec<Vec<String>>,
    email: &str,
    cache_dir: PathBuf,
    staging: bool,
    alpn: Vec<Vec<u8>>,
) -> AcmeAcceptor {
    let mut resolvers = Vec::new();
    for domains in certificates {
        let mut state = AcmeConfig::new(domains.clone())
            .contact_push(format!("mailto:{email}"))
            .cache(DirCache::new(cache_dir.clone()))
            .directory_lets_encrypt(!staging)
            .state();
        resolvers.push((domains.clone(), state.resolver()));

        let name = domains.join(",");
        tokio::spawn(async move {
            loop {
                match state.next().await {
                    Some(Ok(ok)) => info!("acme event for {}: {:?}", name, ok),
                    Some(Err(err)) => error!("acme error for {}: {:?}", name, err),
                    None => break,
                }
            }
        });
    }
    let resolver = Arc::new(SniResolver { resolvers });

    let mut serve = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(resolver.clone());
    serve.alpn_protocols = alpn;
    let mut challenge = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    challenge.alpn_protocols = vec![ACME_TLS_ALPN_NAME.to_vec()];

    AcmeAcceptor {
        serve: Arc::new(serve),
        challenge: Arc::new(challenge),
    }
}

#[derive(Debug)]
struct SniResolver {
    resolvers: Vec<(Vec<String>, Arc<ResolvesServerCertAcme>)>,
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        let name = hello.server_name().map(str::to_ascii_lowercase);
        let (_, resolver) = name
            .and_then(|name| {
                self.resolvers
                    .iter()
                    .find(|(domains, _)| domains.contains(&name))
            })
            .unwrap_or(&self.resolvers[0]);
        resolver.resolve(hello)
    }
}

impl AcmeAcceptor {
    pub async fn accept(&self, tcp: TcpStream) -> io::Result<Option<TlsStream>> {
        tokio::time::timeout(HANDSHAKE_TIMEOUT, self.handshake(tcp))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "tls handshake timed out"))?
    }

    async fn handshake(&self, tcp: TcpStream) -> io::Result<Option<TlsStream>> {
        let handshake = LazyConfigAcceptor::new(Acceptor::default(), tcp.compat()).await?;
        if is_tls_alpn_challenge(&handshake.client_hello()) {
            let _ = handshake.into_stream(self.challenge.clone()).await?;
            return Ok(None);
        }
        let tls = handshake.into_stream(self.serve.clone()).await?;
        Ok(Some(tls.compat()))
    }
}

impl<S: Send + 'static> axum_server::accept::Accept<TcpStream, S> for AcmeAcceptor {
    type Stream = TlsStream;
    type Service = S;
    type Future = Pin<Box<dyn Future<Output = io::Result<(TlsStream, S)>> + Send>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let acceptor = self.clone();
        Box::pin(async move {
            match AcmeAcceptor::accept(&acceptor, stream).await? {
                Some(tls) => Ok((tls, service)),
                None => Err(io::Error::other("TLS-ALPN-01 validation request")),
            }
        })
    }
}
