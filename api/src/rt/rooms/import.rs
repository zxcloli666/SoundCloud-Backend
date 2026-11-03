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
        let page = fetch(&http, api, token, &cursor).await?;
        moved += rooms
            .put_page(&page.keys)
            .await
            .map_err(|error| anyhow::anyhow!("restoring a handoff page failed: {error:?}"))?;
        if page.cursor == LAST_CURSOR {
            return Ok(moved);
        }
        cursor = page.cursor;
    }
}

async fn fetch(
    http: &wreq::Client,
    api: &Url,
    token: &Secret<String>,
    cursor: &str,
) -> anyhow::Result<Page> {
    let url = format!(
        "{}/admin/rooms/handoff?cursor={cursor}&count={PAGE_COUNT}",
        api.as_str().trim_end_matches('/')
    );
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match ask(http, &url, token).await {
            Ok(page) => return Ok(page),
            Err(error) => {
                last = format!("{error:#}");
                tracing::warn!(attempt, error = %last, "rooms handoff page failed");
                tokio::time::sleep(RETRY_AFTER).await;
            }
        }
    }
    bail!("rooms handoff failed after {ATTEMPTS} attempts: {last}")
}

pub(super) async fn ask(
    http: &wreq::Client,
    url: &str,
    token: &Secret<String>,
) -> anyhow::Result<Page> {
    let response = http
        .post(url)
        .header("x-admin-token", token.expose())
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
