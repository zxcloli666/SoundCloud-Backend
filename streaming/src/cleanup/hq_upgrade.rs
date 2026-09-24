use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use tracing::{debug, info, warn};

use crate::db::postgres::PgPool;
use crate::stream::anon::AnonClient;
use crate::stream::cookies_pool::CookiesPool;
use crate::stream::storage::StorageClient;

const TICK: Duration = Duration::from_secs(2 * 60);
const BATCH: i64 = 5;
const RETRY_COOLDOWN_SEC: i64 = 6 * 60 * 60;
const PER_TRACK_GAP: Duration = Duration::from_millis(500);
const UPGRADE_DEADLINE: Duration = Duration::from_secs(180);

type UpgradeOutcome = Result<bool, Box<dyn std::error::Error + Send + Sync>>;

async fn within_deadline<F>(deadline: Duration, urn: &str, work: F) -> UpgradeOutcome
where
    F: std::future::Future<Output = UpgradeOutcome>,
{
    match tokio::time::timeout(deadline, work).await {
        Ok(outcome) => outcome,
        Err(_) => {
            warn!(urn = %urn, "[hq-upgrade] deadline {deadline:?} exceeded");
            Ok(false)
        }
    }
}

pub fn spawn_hq_upgrade_task(
    pg: PgPool,
    anon: Arc<AnonClient>,
    cookies: Option<Arc<CookiesPool>>,
    storage: Arc<StorageClient>,
    decryptor: Option<Arc<decrypt::Engine>>,
    http_client: wreq::Client,
    sc_proxy_url: String,
) {
    if !storage.enabled() {
        info!("[hq-upgrade] storage disabled, skipping");
        return;
    }
    let Some(engine) = decryptor else {
        info!("[hq-upgrade] decryptor not configured, skipping");
        return;
    };
    info!(
        tick_sec = TICK.as_secs(),
        batch = BATCH,
        "[hq-upgrade] starting"
    );

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let urns = match pg
                .pick_hq_upgrade_candidates(BATCH, RETRY_COOLDOWN_SEC)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    warn!("[hq-upgrade] pick failed: {e}");
                    continue;
                }
            };
            if urns.is_empty() {
                continue;
            }
            for urn in urns {
                tokio::time::sleep(PER_TRACK_GAP).await;
                let res = within_deadline(
                    UPGRADE_DEADLINE,
                    &urn,
                    upgrade_one(
                        &urn,
                        &anon,
                        cookies.as_ref(),
                        &engine,
                        &http_client,
                        &sc_proxy_url,
                        &storage,
                    ),
                )
                .await;
                match res {
                    Ok(true) => debug!(urn = %urn, "[hq-upgrade] uploaded"),
                    Ok(false) => {
                        if let Err(e) = pg.mark_hq_upgrade_failed(&urn).await {
                            warn!(urn = %urn, "[hq-upgrade] mark_failed: {e}");
                        }
                    }
                    Err(e) => warn!(urn = %urn, "[hq-upgrade] error: {e}"),
                }
            }
        }
    });
}

async fn upgrade_one(
    track_urn: &str,
    anon: &AnonClient,
    cookies: Option<&Arc<CookiesPool>>,
    engine: &decrypt::Engine,
    http_client: &wreq::Client,
    sc_proxy_url: &str,
    storage: &StorageClient,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let src = match anon.resolve_restricted(track_urn, true).await {
        Ok(Some(v)) => Some(v),
        Ok(None) => None,
        Err(e) => {
            debug!(urn = %track_urn, "[hq-upgrade] anon resolve failed: {e}");
            None
        }
    };
    let src = match src {
        Some(s) => s,
        None => match cookies {
            Some(c) => match c.resolve_restricted(track_urn, true).await {
                Ok(Some(v)) => v,
                Ok(None) => return Ok(false),
                Err(e) => {
                    debug!(urn = %track_urn, "[hq-upgrade] cookies resolve failed: {e}");
                    return Ok(false);
                }
            },
            None => return Ok(false),
        },
    };

    if !src.is_hq {
        debug!(urn = %track_urn, "[hq-upgrade] source not marked hq, skipping");
        return Ok(false);
    }

    let fetcher: Arc<dyn decrypt::Fetcher> = Arc::new(crate::stream::decrypt_fetch::ProxyFetcher {
        client: http_client.clone(),
        proxy_url: sc_proxy_url.to_string(),
    });
    let mut stream = engine
        .process_stream(&src.manifest, &src.token, fetcher)
        .await?;

    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(b) => buf.extend_from_slice(&b),
            Err(e) => return Err(Box::new(e)),
        }
    }
    if buf.len() < 32 * 1024 {
        debug!(urn = %track_urn, bytes = buf.len(), "[hq-upgrade] too small, skipping");
        return Ok(false);
    }
    storage.upload_in_background_with_quality(track_urn.to_string(), Bytes::from(buf), "hq");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn an_upgrade_that_never_finishes_frees_the_worker_for_the_next_track() {
        let stuck = async {
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            Ok(true)
        };

        let started = tokio::time::Instant::now();
        let outcome = within_deadline(UPGRADE_DEADLINE, "soundcloud:tracks:1", stuck).await;

        assert!(
            matches!(outcome, Ok(false)),
            "a track that never finishes must be written off, not carried out of the loop \
             as an error the caller treats differently"
        );
        assert_eq!(
            started.elapsed(),
            UPGRADE_DEADLINE,
            "the worker waited longer than the budget, so one bad track still stalls the batch"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_upgrade_that_finishes_in_time_is_passed_through_untouched() {
        let quick = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(true)
        };

        let outcome = within_deadline(UPGRADE_DEADLINE, "soundcloud:tracks:2", quick).await;

        assert!(matches!(outcome, Ok(true)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_inside_the_budget_stays_a_failure() {
        let failing = async { Err("upstream refused".into()) };

        let outcome = within_deadline(UPGRADE_DEADLINE, "soundcloud:tracks:3", failing).await;

        assert!(
            outcome.is_err(),
            "a real error must not be flattened into an ordinary skip"
        );
    }
}
