use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use tracing::{debug, info, warn};

use crate::db::postgres::PgPool;
use crate::stream::anon::AnonClient;
use crate::stream::cookies_pool::CookiesPool;
use crate::stream::quality;
use crate::stream::restricted::RestrictedSource;
use crate::stream::storage::StorageClient;

const TICK: Duration = Duration::from_secs(2 * 60);
const BATCH: i64 = 10;
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
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
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
    anon: &Arc<AnonClient>,
    cookies: Option<&Arc<CookiesPool>>,
    engine: &decrypt::Engine,
    http_client: &wreq::Client,
    sc_proxy_url: &str,
    storage: &StorageClient,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(pool) = cookies.filter(|pool| pool.serves_uq())
        && let Ok(Some(result)) = pool.get_stream(track_urn, true).await
    {
        return Ok(upload(
            storage,
            track_urn,
            result.data.to_vec(),
            result.quality,
        ));
    }

    if let Some(src) = restricted_source(track_urn, anon, cookies).await
        && above_sq(src.quality)
    {
        let data = decrypt_all(&src, engine, http_client, sc_proxy_url).await?;
        return Ok(upload(storage, track_urn, data, src.quality));
    }

    match anon.get_stream(track_urn, quality::HQ).await {
        Ok(Some(result)) => Ok(upload(
            storage,
            track_urn,
            result.data.to_vec(),
            result.quality,
        )),
        Ok(None) => Ok(false),
        Err(e) => {
            debug!(urn = %track_urn, "[hq-upgrade] anon stream failed: {e}");
            Ok(false)
        }
    }
}

fn above_sq(quality: &str) -> bool {
    quality::rank(quality) > quality::rank(quality::SQ)
}

async fn restricted_source(
    track_urn: &str,
    anon: &AnonClient,
    cookies: Option<&Arc<CookiesPool>>,
) -> Option<RestrictedSource> {
    match anon.resolve_restricted(track_urn, true).await {
        Ok(Some(v)) => return Some(v),
        Ok(None) => {}
        Err(e) => debug!(urn = %track_urn, "[hq-upgrade] anon resolve failed: {e}"),
    }
    match cookies?.resolve_restricted(track_urn, true).await {
        Ok(v) => v,
        Err(e) => {
            debug!(urn = %track_urn, "[hq-upgrade] cookies resolve failed: {e}");
            None
        }
    }
}

async fn decrypt_all(
    src: &RestrictedSource,
    engine: &decrypt::Engine,
    http_client: &wreq::Client,
    sc_proxy_url: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let fetcher: Arc<dyn decrypt::Fetcher> = Arc::new(crate::stream::decrypt_fetch::ProxyFetcher {
        client: http_client.clone(),
        proxy_url: sc_proxy_url.to_string(),
    });
    let mut stream = engine
        .process_stream(&src.manifest, &src.token, fetcher)
        .await?;

    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk?);
    }
    Ok(buf)
}

fn upload(storage: &StorageClient, track_urn: &str, data: Vec<u8>, quality: &'static str) -> bool {
    if data.len() < 32 * 1024 {
        debug!(urn = %track_urn, bytes = data.len(), "[hq-upgrade] too small, skipping");
        return false;
    }
    storage.upload_in_background_with_quality(track_urn.to_string(), Bytes::from(data), quality);
    true
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
