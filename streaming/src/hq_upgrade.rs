mod tiers;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use tracing::{debug, info, warn};

use crate::db::postgres::PgPool;
use crate::db::postgres::upgrade::UpgradeTier;
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

    let upgrader = Upgrader {
        pg,
        anon,
        cookies,
        storage,
        engine,
        http_client,
        sc_proxy_url,
    };
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut turn: u64 = 0;
        loop {
            ticks.tick().await;
            let tiers = tiers::for_turn(turn);
            turn = turn.wrapping_add(1);
            upgrader.run_batch(quality::SQ, &tiers).await;
            if upgrader.uq_live() {
                upgrader.run_batch(quality::HQ, &tiers).await;
            }
        }
    });
}

struct Upgrader {
    pg: PgPool,
    anon: Arc<AnonClient>,
    cookies: Option<Arc<CookiesPool>>,
    storage: Arc<StorageClient>,
    engine: Arc<decrypt::Engine>,
    http_client: wreq::Client,
    sc_proxy_url: String,
}

impl Upgrader {
    fn uq_live(&self) -> bool {
        self.cookies.as_ref().is_some_and(|pool| pool.serves_uq())
    }

    async fn run_batch(&self, stored: &'static str, tiers: &[UpgradeTier]) {
        if !self.storage.has_free_upload_slot() {
            debug!(
                stored,
                "[hq-upgrade] every storage upload slot is busy, waiting for the next tick"
            );
            return;
        }
        for urn in self.pick(stored, tiers).await {
            tokio::time::sleep(PER_TRACK_GAP).await;
            let outcome =
                within_deadline(UPGRADE_DEADLINE, &urn, self.upgrade_one(&urn, stored)).await;
            match outcome {
                Ok(true) => debug!(urn = %urn, "[hq-upgrade] uploaded"),
                Ok(false) => {
                    if let Err(e) = self.pg.mark_hq_upgrade_failed(&urn).await {
                        warn!(urn = %urn, "[hq-upgrade] mark_failed: {e}");
                    }
                }
                Err(e) => warn!(urn = %urn, "[hq-upgrade] error: {e}"),
            }
        }
    }

    async fn pick(&self, stored: &'static str, tiers: &[UpgradeTier]) -> Vec<String> {
        let mut urns: Vec<String> = Vec::new();
        let mut round = Vec::new();
        for &tier in tiers {
            let left = BATCH - urns.len() as i64;
            match self
                .pg
                .pick_upgrade_candidates(stored, tier, left, RETRY_COOLDOWN_SEC)
                .await
            {
                Ok(pick) => {
                    round.push(tiers::Picked {
                        tier,
                        count: pick.urns.len(),
                        waiting_seconds: pick.waiting_seconds,
                    });
                    urns.extend(pick.urns);
                }
                Err(e) => warn!("[hq-upgrade] pick {stored} {} failed: {e}", tier.label()),
            }
        }
        tiers::report(stored, &round);
        urns
    }

    async fn upgrade_one(&self, track_urn: &str, stored: &'static str) -> UpgradeOutcome {
        if self.uq_live()
            && let Some(pool) = &self.cookies
            && let Ok(Some(result)) = pool.get_stream(track_urn, true).await
        {
            return Ok(self.upload(track_urn, result.data, result.quality));
        }
        if stored != quality::SQ {
            return Ok(false);
        }

        if let Some(src) = self.restricted_source(track_urn).await {
            let data = self.decrypt_all(&src).await?;
            return Ok(self.upload(track_urn, data, src.quality));
        }

        match self.anon.get_stream(track_urn, quality::HQ).await {
            Ok(Some(result)) => Ok(self.upload(track_urn, result.data, result.quality)),
            Ok(None) => Ok(false),
            Err(e) => {
                debug!(urn = %track_urn, "[hq-upgrade] anon stream failed: {e}");
                Ok(false)
            }
        }
    }

    async fn restricted_source(&self, track_urn: &str) -> Option<RestrictedSource> {
        match self
            .anon
            .resolve_restricted(track_urn, true, quality::HQ)
            .await
        {
            Ok(Some(v)) => return Some(v),
            Ok(None) => {}
            Err(e) => debug!(urn = %track_urn, "[hq-upgrade] anon resolve failed: {e}"),
        }
        if !self.uq_live() {
            return None;
        }
        match self
            .cookies
            .as_ref()?
            .resolve_restricted(track_urn, true, quality::HQ)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                debug!(urn = %track_urn, "[hq-upgrade] cookies resolve failed: {e}");
                None
            }
        }
    }

    async fn decrypt_all(
        &self,
        src: &RestrictedSource,
    ) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
        let fetcher: Arc<dyn decrypt::Fetcher> =
            Arc::new(crate::stream::decrypt_fetch::ProxyFetcher {
                client: self.http_client.clone(),
                proxy_url: self.sc_proxy_url.clone(),
            });
        let mut stream = self
            .engine
            .process_stream(&src.manifest, &src.token, fetcher)
            .await?;

        let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk?);
        }
        buf.shrink_to_fit();
        Ok(Bytes::from(buf))
    }

    fn upload(&self, track_urn: &str, data: Bytes, quality: &'static str) -> bool {
        if data.len() < 32 * 1024 {
            debug!(urn = %track_urn, bytes = data.len(), "[hq-upgrade] too small, skipping");
            return false;
        }
        self.storage
            .upload_in_background_with_quality(track_urn.to_string(), data, quality);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_reaches_a_cookies_account_only_while_the_pool_is_alive() {
        let source = crate::source_tree::read("src/hq_upgrade.rs");
        let body = source
            .split_once("async fn restricted_source(")
            .expect("the upgrader still resolves restricted tracks")
            .1;
        let body = body.split_once("\n    }\n").expect("the function ends").0;
        let gate = body
            .find("if !self.uq_live()")
            .expect("restricted_source no longer checks that the pool is alive");
        let cookies = body
            .find(".cookies")
            .expect("restricted_source no longer reads cookies");
        assert!(
            gate < cookies,
            "about a million queued tracks would each spend a cookies account request \
             while those accounts are dead"
        );
    }

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
