use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine;
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::catalog_read::PublicCatalogReader;

const APIV2: &str = "https://api-v2.soundcloud.com/";
const HOME: &str = r#"<script>window.__sc_hydration = [{"hydratable":"apiClient","data":{"id":"stub-client"}}]</script>"#;

pub(super) struct SoundCloud {
    proxy_url: String,
    asked: Arc<Mutex<Vec<String>>>,
    server: JoinHandle<anyhow::Result<()>>,
}

impl SoundCloud {
    pub(super) async fn answering(answers: Vec<(&'static str, String)>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let proxy_url = format!("http://{}/", listener.local_addr()?);
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked);
        let server: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let mut answers = answers.into_iter();
            loop {
                let (mut stream, _) = listener.accept().await?;
                let target = read_target(&mut stream).await?;
                if !target.starts_with(APIV2) {
                    respond(&mut stream, "200 OK", HOME).await?;
                    continue;
                }
                seen.lock().await.push(target);
                let (status, body) = answers
                    .next()
                    .unwrap_or_else(|| ("404 Not Found", "{}".to_owned()));
                respond(&mut stream, status, &body).await?;
            }
        });
        Ok(Self {
            proxy_url,
            asked,
            server,
        })
    }

    pub(super) async fn asked(&self) -> usize {
        self.asked.lock().await.len()
    }

    pub(super) fn client(&self) -> anyhow::Result<sc_transport::ScClient> {
        Ok(sc_transport::ScClient::new(&sc_transport::ScConfig {
            proxy_url: self.proxy_url.clone(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some("https://soundcloud.com/".to_owned()),
        })?)
    }

    pub(super) fn reader(&self, pool: &PgPool) -> anyhow::Result<PublicCatalogReader> {
        Ok(PublicCatalogReader::new(self.client()?, pool.clone()))
    }
}

impl Drop for SoundCloud {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn read_target(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || request.len().saturating_add(read) > 16 * 1024 {
            return Err(anyhow::anyhow!("the stub received an invalid request"));
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let request = String::from_utf8(request)?;
    let encoded = request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("x-target"))
        .map(|(_, value)| value.trim())
        .ok_or_else(|| anyhow::anyhow!("the request names no target"))?;
    Ok(String::from_utf8(
        base64::engine::general_purpose::STANDARD.decode(encoded)?,
    )?)
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) -> anyhow::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

pub(super) struct GoneRelay {
    pub(super) calls: Arc<AtomicUsize>,
}

impl sc_transport::RelayTransport for GoneRelay {
    fn fetch<'a>(
        &'a self,
        _request: &'a call_relay::Request,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<call_relay::Response, call_relay::Error>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err(call_relay::Error::Disabled) })
    }

    fn call_method_rotated<'a>(
        &'a self,
        _method_id: &'a str,
        _script: &'a str,
        _inputs: sc_transport::Bytes,
        _region_rotation: i32,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<sc_transport::Bytes, call_relay::Error>>
                + Send
                + 'a,
        >,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(sc_transport::Bytes::from_static(
                br#"{"ok":false,"reason":"gone"}"#,
            ))
        })
    }
}
