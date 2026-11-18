use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use catalog_sources::{SourceError, SourceResult};
use futures::StreamExt;
use wreq::Method;
use wreq::header::HeaderMap;

use super::pool::ProxyPool;
use super::ytdlp::Exit;
use crate::handlers::video_accounts::Fetch;

const PAGE_LIMIT: usize = 4 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TIMEOUT: Duration = Duration::from_secs(30);
const EGRESS_TAG: &str = "x-egress-tag";
const RUSSIAN_EXIT: &str = "geo-ru";

pub struct Pooled {
    direct: wreq::Client,
    pool: Arc<ProxyPool>,
}

impl Pooled {
    pub(crate) fn new(pool: Arc<ProxyPool>) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            direct: client(None)?,
            pool,
        })
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        mut headers: HeaderMap,
        body: Option<Bytes>,
    ) -> SourceResult<Bytes> {
        let exit = if headers
            .get(EGRESS_TAG)
            .is_some_and(|tag| tag == RUSSIAN_EXIT)
        {
            Exit::Russia
        } else {
            Exit::Anywhere
        };
        headers.remove(EGRESS_TAG);
        self.pool
            .routed(exit, |proxy| {
                let (method, headers, body) = (method.clone(), headers.clone(), body.clone());
                async move {
                    let http = match proxy {
                        None => self.direct.clone(),
                        Some(proxy) => client(Some(&proxy)).map_err(|_| {
                            SourceError::Unreachable("proxy client did not build".to_owned())
                        })?,
                    };
                    let mut request = http.request(method, url).headers(headers);
                    if let Some(body) = body {
                        request = request.body(body);
                    }
                    let response = request
                        .send()
                        .await
                        .map_err(|error| SourceError::Unreachable(error.to_string()))?;
                    read(response).await
                }
            })
            .await
    }
}

impl Fetch for Pooled {
    async fn get(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
        self.send(Method::GET, url, headers, None).await
    }

    async fn post(&self, url: &str, headers: HeaderMap, body: Bytes) -> SourceResult<Bytes> {
        self.send(Method::POST, url, headers, Some(body)).await
    }
}

fn client(proxy: Option<&str>) -> anyhow::Result<wreq::Client> {
    let mut builder = sc_fingerprint::builder(None)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT);
    if let Some(proxy) = proxy {
        builder = builder.proxy(wreq::Proxy::all(proxy)?);
    }
    Ok(builder.build()?)
}

async fn read(response: wreq::Response) -> SourceResult<Bytes> {
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|length| length > PAGE_LIMIT as u64)
    {
        return Err(SourceError::Oversized { limit: PAGE_LIMIT });
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| SourceError::Unreachable(error.to_string()))?;
        if bytes.len() + chunk.len() > PAGE_LIMIT {
            return Err(SourceError::Oversized { limit: PAGE_LIMIT });
        }
        bytes.extend_from_slice(&chunk);
    }
    if status >= 400 {
        return Err(SourceError::Status {
            status,
            body: String::from_utf8_lossy(&bytes).chars().take(200).collect(),
            retry_after_seconds: None,
        });
    }
    Ok(Bytes::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socks_proxies_build_a_client_and_garbage_does_not() {
        assert!(client(Some("socks5://user:pass@127.0.0.1:1080")).is_ok());
        assert!(client(Some("socks5h://user:pass@127.0.0.1:1080")).is_ok());
        assert!(client(Some("not a proxy")).is_err());
    }

    #[tokio::test]
    async fn a_dead_direct_target_falls_through_dead_proxies_without_leaking_their_urls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxies.txt");
        std::fs::write(
            &path,
            "socks5://user:secret@127.0.0.1:1 tags=dc\nsocks5://user:secret@127.0.0.1:2 tags=dc\n",
        )
        .unwrap();
        let pooled = Pooled::new(Arc::new(ProxyPool::new(Some(path)))).unwrap();

        let error = pooled
            .get("http://127.0.0.1:3/", HeaderMap::new())
            .await
            .unwrap_err();

        assert!(matches!(error, SourceError::Unreachable(_)));
        assert!(!error.to_string().contains("secret"));
    }
}
