use std::time::Duration;

use anyhow::{Context, bail};
use redact::Secret;
use url::Url;

use crate::modules::rooms::RoomsService;
use crate::modules::rooms::store::Page;

pub const PAGE_COUNT: usize = 200;
pub const ATTEMPTS: u32 = 5;
pub const RETRY_AFTER: Duration = Duration::from_secs(2);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const LAST_CURSOR: &str = "0";

pub async fn hand_off(
    api: &Url,
    token: &Secret<String>,
    rooms: &RoomsService,
) -> anyhow::Result<usize> {
    let http = wreq::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("the handoff http client does not build")?;
    let mut cursor = LAST_CURSOR.to_owned();
    let mut moved = 0;
    loop {
        let (next, count) = move_page(&http, api, token, rooms, &cursor).await?;
        moved += count;
        if next == LAST_CURSOR {
            return Ok(moved);
        }
        cursor = next;
    }
}

pub(super) async fn move_page(
    http: &wreq::Client,
    api: &Url,
    token: &Secret<String>,
    rooms: &RoomsService,
    cursor: &str,
) -> anyhow::Result<(String, usize)> {
    let base = api.as_str().trim_end_matches('/');
    let page = fetch(http, base, token, cursor).await?;
    let count = rooms
        .put_page(&page.keys)
        .await
        .map_err(|error| anyhow::anyhow!("restoring a handoff page failed: {error:?}"))?;
    acknowledge(http, base, token, &page).await?;
    Ok((page.cursor, count))
}

async fn fetch(
    http: &wreq::Client,
    base: &str,
    token: &Secret<String>,
    cursor: &str,
) -> anyhow::Result<Page> {
    let url = format!("{base}/admin/rooms/handoff?cursor={cursor}&count={PAGE_COUNT}");
    with_retries("page", || ask(http, &url, token)).await
}

async fn acknowledge(
    http: &wreq::Client,
    base: &str,
    token: &Secret<String>,
    page: &Page,
) -> anyhow::Result<()> {
    if page.keys.is_empty() {
        return Ok(());
    }
    let url = format!("{base}/admin/rooms/handoff/ack");
    let keys: Vec<&str> = page.keys.iter().map(|moved| moved.key.as_str()).collect();
    with_retries("ack", || tell(http, &url, token, &keys)).await
}

async fn with_retries<T, F, Fut>(what: &str, mut call: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match call().await {
            Ok(done) => return Ok(done),
            Err(error) => {
                last = format!("{error:#}");
                tracing::warn!(attempt, error = %last, "rooms handoff {what} failed");
                tokio::time::sleep(RETRY_AFTER).await;
            }
        }
    }
    bail!("rooms handoff {what} failed after {ATTEMPTS} attempts: {last}")
}

pub(super) async fn ask(
    http: &wreq::Client,
    url: &str,
    token: &Secret<String>,
) -> anyhow::Result<Page> {
    let response = http
        .post(url)
        .header("x-handoff-token", token.expose())
        .send()
        .await
        .context("the api is unreachable")?;
    let status = response.status();
    if !status.is_success() {
        bail!("the api answered {status}");
    }
    let body = response.bytes().await.context("the page body broke")?;
    serde_json::from_slice(&body).context("the page is not valid")
}

async fn tell(
    http: &wreq::Client,
    url: &str,
    token: &Secret<String>,
    keys: &[&str],
) -> anyhow::Result<()> {
    let response = http
        .post(url)
        .header("x-handoff-token", token.expose())
        .json(&serde_json::json!({ "keys": keys }))
        .send()
        .await
        .context("the api is unreachable")?;
    let status = response.status();
    if !status.is_success() {
        bail!("the api answered {status}");
    }
    Ok(())
}
