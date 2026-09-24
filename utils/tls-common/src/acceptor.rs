use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::http::Request;
use axum_server::accept::Accept;
use tokio::net::TcpStream;
use tower::Service;

use crate::proxy::read_proxy_v1;

const PROXY_SIGNATURE: &[u8; 6] = b"PROXY ";
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct TrustedProxies {
    pub cidrs: Arc<Vec<crate::config::IpCidr>>,
    pub resolved: Arc<RwLock<Vec<IpAddr>>>,
}

impl TrustedProxies {
    pub fn contains(&self, ip: IpAddr) -> bool {
        if self.cidrs.iter().any(|c| c.contains(ip)) {
            return true;
        }
        self.resolved
            .read()
            .map(|r| r.contains(&ip))
            .unwrap_or(false)
    }
}

#[derive(Clone)]
pub(crate) struct ConnectInfoAcceptor<A> {
    pub inner: A,
    pub proxy_protocol: bool,
    pub proxy_trusted: TrustedProxies,
}

impl<A, S> Accept<TcpStream, S> for ConnectInfoAcceptor<A>
where
    A: Accept<TcpStream, ConnectInfoService<S>> + Clone + Send + Sync + 'static,
    A::Future: Send + 'static,
    A::Stream: Send + 'static,
    A::Service: Send + 'static,
    S: Send + 'static,
{
    type Stream = A::Stream;
    type Service = A::Service;
    type Future = Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let inner = self.inner.clone();
        let proxy_protocol = self.proxy_protocol;
        let proxy_trusted = self.proxy_trusted.clone();
        Box::pin(async move {
            let accepted = async {
                let mut stream = stream;
                let peer = stream.peer_addr()?;
                let real_addr = if proxy_protocol && peek_proxy_signature(&stream).await? {
                    let advertised = read_proxy_v1(&mut stream).await?;
                    if proxy_trusted.contains(peer.ip()) {
                        advertised
                    } else {
                        peer
                    }
                } else {
                    peer
                };
                let service = ConnectInfoService {
                    inner: service,
                    addr: real_addr,
                };
                inner.accept(stream, service).await
            };
            tokio::time::timeout(ACCEPT_TIMEOUT, accepted)
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "connection preface timed out")
                })?
        })
    }
}

async fn peek_proxy_signature(stream: &TcpStream) -> io::Result<bool> {
    let mut sig = [0u8; 6];
    let mut observed = 0;
    loop {
        let count = stream.peek(&mut sig).await?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before request preface",
            ));
        }
        let compared = count.min(PROXY_SIGNATURE.len());
        if sig[..compared] != PROXY_SIGNATURE[..compared] {
            return Ok(false);
        }
        if count >= PROXY_SIGNATURE.len() {
            return Ok(true);
        }
        if count == observed {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        observed = count;
    }
}

#[derive(Clone)]
pub(crate) struct ConnectInfoService<S> {
    pub inner: S,
    pub addr: SocketAddr,
}

impl<S, B> Service<Request<B>> for ConnectInfoService<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        req.extensions_mut().insert(ConnectInfo(self.addr));
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    use super::*;

    async fn socket_pair() -> io::Result<(TcpStream, TcpStream)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let client = TcpStream::connect(address);
        let server = listener.accept();
        let (client, (server, _)) = tokio::try_join!(client, server)?;
        Ok((client, server))
    }

    #[tokio::test]
    async fn fragmented_post_is_not_a_proxy_header() -> io::Result<()> {
        let (mut client, server) = socket_pair().await?;
        let detection = tokio::spawn(async move { peek_proxy_signature(&server).await });

        client.write_all(b"P").await?;
        tokio::task::yield_now().await;
        client.write_all(b"OST /").await?;

        assert!(!detection.await??);
        Ok(())
    }

    #[tokio::test]
    async fn fragmented_proxy_signature_is_recognized() -> io::Result<()> {
        let (mut client, server) = socket_pair().await?;
        let detection = tokio::spawn(async move { peek_proxy_signature(&server).await });

        client.write_all(b"PRO").await?;
        tokio::task::yield_now().await;
        client.write_all(b"XY ").await?;

        assert!(detection.await??);
        Ok(())
    }
}
