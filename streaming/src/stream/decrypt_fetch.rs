use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::Arc;
use wreq::Client;

use super::proxy::{fetch_get_bytes, fetch_post_bytes};
use super::restricted::RestrictedSource;

const WHOLE_AT_MOST: usize = 32 << 20;

pub(crate) struct ProxyFetcher {
    pub client: Client,
    pub proxy_url: String,
}

pub(crate) async fn decrypt_whole(
    engine: &decrypt::Engine,
    client: &Client,
    proxy_url: &str,
    src: &RestrictedSource,
) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
    let fetcher: Arc<dyn decrypt::Fetcher> = Arc::new(ProxyFetcher {
        client: client.clone(),
        proxy_url: proxy_url.to_owned(),
    });
    let mut stream = engine
        .process_stream(&src.manifest, &src.token, fetcher)
        .await?;
    let mut whole: Vec<u8> = Vec::with_capacity(8 << 20);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if whole.len() + chunk.len() > WHOLE_AT_MOST {
            return Err(format!("decrypted track passed {WHOLE_AT_MOST} bytes").into());
        }
        whole.extend_from_slice(&chunk);
    }
    whole.shrink_to_fit();
    Ok(Bytes::from(whole))
}

fn map(headers: Vec<(String, String)>) -> HashMap<String, String> {
    headers.into_iter().collect()
}

impl decrypt::Fetcher for ProxyFetcher {
    fn get(
        &self,
        url: String,
        headers: Vec<(String, String)>,
    ) -> BoxFuture<'static, Result<Bytes, decrypt::Error>> {
        let client = self.client.clone();
        let proxy = self.proxy_url.clone();
        Box::pin(async move {
            fetch_get_bytes(&client, &proxy, &url, map(headers), false)
                .await
                .map(|(b, _)| b)
                .map_err(|e| decrypt::Error::Fetch(e.to_string()))
        })
    }

    fn post(
        &self,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> BoxFuture<'static, Result<Bytes, decrypt::Error>> {
        let client = self.client.clone();
        let proxy = self.proxy_url.clone();
        Box::pin(async move {
            fetch_post_bytes(&client, &proxy, &url, map(headers), body)
                .await
                .map(|(b, _)| b)
                .map_err(|e| decrypt::Error::Fetch(e.to_string()))
        })
    }
}
