use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::Uri;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;
use tracing::{debug, warn};

const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct ClientIdentity {
    pub ca: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl ClientIdentity {
    pub fn from_env(prefix: &str) -> Option<Self> {
        let read = |suffix: &str| {
            std::env::var(format!("{prefix}_TLS_{suffix}"))
                .ok()
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        match (read("CA"), read("CERT"), read("KEY")) {
            (Some(ca), Some(cert), Some(key)) => Some(Self { ca, cert, key }),
            (None, None, None) => None,
            _ => {
                warn!(
                    prefix,
                    "client TLS needs _TLS_CA, _TLS_CERT and _TLS_KEY together; running without it"
                );
                None
            }
        }
    }

    pub fn client_config(&self) -> io::Result<ClientConfig> {
        let mut roots = RootCertStore::empty();
        for cert in CertificateDer::pem_file_iter(&self.ca).map_err(invalid)? {
            roots.add(cert.map_err(invalid)?).map_err(invalid)?;
        }
        let chain = CertificateDer::pem_file_iter(&self.cert)
            .map_err(invalid)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(invalid)?;
        let key = PrivateKeyDer::from_pem_file(&self.key).map_err(invalid)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(invalid)?
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, key)
            .map_err(invalid)?;
        config.alpn_protocols = vec![b"h2".to_vec()];
        Ok(config)
    }
}

pub fn tunnel_url(url: &str, identity: ClientIdentity) -> io::Result<String> {
    let uri: Uri = url.parse().map_err(invalid)?;
    let host = uri
        .host()
        .ok_or_else(|| invalid("url has no host"))?
        .to_owned();
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("http") {
            80
        } else {
            443
        });
    identity.client_config()?;
    let local = start_tunnel(host, port, identity)?;
    Ok(format!("http://{local}"))
}

fn start_tunnel(host: String, port: u16, identity: ClientIdentity) -> io::Result<SocketAddr> {
    let runtime = tokio::runtime::Handle::try_current().map_err(invalid)?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let local = listener.local_addr()?;
    let listener = {
        let _guard = runtime.enter();
        TcpListener::from_std(listener)?
    };
    runtime.spawn(accept_loop(listener, host, port, identity));
    Ok(local)
}

async fn accept_loop(listener: TcpListener, host: String, port: u16, identity: ClientIdentity) {
    let host = Arc::new(host);
    let identity = Arc::new(identity);
    loop {
        match listener.accept().await {
            Ok((local, _)) => {
                let host = host.clone();
                let identity = identity.clone();
                tokio::spawn(async move {
                    if let Err(error) = relay(local, &host, port, &identity).await {
                        debug!(%error, host = %host, "mTLS tunnel connection failed");
                    }
                });
            }
            Err(error) => {
                warn!(%error, "mTLS tunnel accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn relay(
    mut local: TcpStream,
    host: &str,
    port: u16,
    identity: &ClientIdentity,
) -> io::Result<()> {
    let connector = TlsConnector::from(Arc::new(identity.client_config()?));
    let name = ServerName::try_from(host.to_owned()).map_err(invalid)?;
    let mut upstream = tokio::time::timeout(UPSTREAM_TIMEOUT, async {
        let tcp = TcpStream::connect((host, port)).await?;
        connector.connect(name, tcp).await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "upstream connect timed out"))??;
    tokio::io::copy_bidirectional(&mut local, &mut upstream).await?;
    Ok(())
}

fn invalid(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}
