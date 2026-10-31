use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use mini_moka::sync::Cache;
use sqlx::PgPool;
use tracing::debug;
use wreq::Client;

use crate::error::AppResult;

const HEAD_CONCURRENCY: usize = 16;
const HEAD_TIMEOUT: Duration = Duration::from_secs(3);
const FOUND_KEPT: Duration = Duration::from_secs(7 * 24 * 3600);
const GONE_KEPT: Duration = Duration::from_secs(24 * 3600);
const REMEMBERED: u64 = 200_000;

pub struct S3VerifierService {
    http: Client,
    storage_url: String,
    pg: PgPool,
    writes_verdicts: bool,
    found: Cache<String, ()>,
    gone: Cache<String, ()>,
}

impl S3VerifierService {
    pub fn new(http: Client, storage_url: String, pg: PgPool, writes_verdicts: bool) -> Arc<Self> {
        let trimmed = storage_url.trim_end_matches('/').to_string();
        Arc::new(Self {
            http,
            storage_url: trimmed,
            pg,
            writes_verdicts,
            found: kept_for(FOUND_KEPT),
            gone: kept_for(GONE_KEPT),
        })
    }

    pub async fn find_missing(&self, sc_track_ids: &[String]) -> AppResult<HashSet<String>> {
        let mut missing: HashSet<String> = HashSet::new();
        if sc_track_ids.is_empty() || self.storage_url.is_empty() {
            return Ok(missing);
        }

        let ttl_cutoff = Utc::now() - chrono::Duration::hours(24);
        let recheck_cutoff = Utc::now() - chrono::Duration::days(7);

        type VerifyMap =
            std::collections::HashMap<String, (Option<DateTime<Utc>>, Option<DateTime<Utc>>)>;
        let rows = sqlx::query_file!(
            "queries/recommendations/s3_verifier/select_verify_rows.sql",
            sc_track_ids
        )
        .fetch_all(&self.pg)
        .await?;
        let mut by_id: VerifyMap = std::collections::HashMap::new();
        for row in rows {
            by_id.insert(row.sc_track_id, (row.s3_verified_at, row.s3_missing_at));
        }

        let mut to_check: Vec<String> = Vec::new();
        for id in sc_track_ids {
            match by_id.get(id) {
                Some((Some(verified), m))
                    if *verified > recheck_cutoff && m.map(|x| x <= *verified).unwrap_or(true) =>
                {
                    continue;
                }
                Some((_, Some(miss))) if *miss > ttl_cutoff => {
                    missing.insert(id.clone());
                }
                _ if self.gone.contains_key(id) => {
                    missing.insert(id.clone());
                }
                _ if self.found.contains_key(id) => {}
                _ => to_check.push(id.clone()),
            }
        }
        if to_check.is_empty() {
            return Ok(missing);
        }

        let checks = stream::iter(to_check.iter().cloned())
            .map(|id| {
                let this = self;
                async move {
                    let found = this.probe(&id).await;
                    (id, found)
                }
            })
            .buffer_unordered(HEAD_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        let mut ok_ids: Vec<String> = Vec::new();
        let mut miss_ids: Vec<String> = Vec::new();
        for (id, found) in checks {
            match found {
                Some(true) => ok_ids.push(id),
                Some(false) => {
                    missing.insert(id.clone());
                    miss_ids.push(id);
                }
                None => {}
            }
        }

        if !miss_ids.is_empty() {
            debug!(
                misses = miss_ids.len(),
                oks = ok_ids.len(),
                "S3 verify result"
            );
        }
        self.remember(ok_ids, miss_ids).await?;
        Ok(missing)
    }

    async fn remember(&self, ok: Vec<String>, missing: Vec<String>) -> AppResult<()> {
        if ok.is_empty() && missing.is_empty() {
            return Ok(());
        }
        for id in &ok {
            self.found.insert(id.clone(), ());
        }
        for id in &missing {
            self.gone.insert(id.clone(), ());
        }
        if !self.writes_verdicts {
            return Ok(());
        }
        let verdicts = wt::TrackS3State {
            ok,
            missing,
            at: wt::now(),
        };
        wt::retry(|| async {
            let mut transaction = self.pg.begin().await?;
            wt::call(&mut transaction, &verdicts).await?;
            transaction.commit().await
        })
        .await?;
        Ok(())
    }

    async fn probe(&self, sc_track_id: &str) -> Option<bool> {
        let key = catalog_ingest::track_object_key(sc_track_id)?;
        let url = format!("{}/{key}", self.storage_url);
        match self.http.head(&url).timeout(HEAD_TIMEOUT).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let verdict = head_verdict(status);
                if verdict.is_none() {
                    debug!(url, status, "HEAD neither found nor gone");
                }
                verdict
            }
            Err(e) => {
                debug!(url, error = %e, "HEAD failed");
                None
            }
        }
    }
}

fn kept_for(ttl: Duration) -> Cache<String, ()> {
    Cache::builder()
        .max_capacity(REMEMBERED)
        .time_to_live(ttl)
        .build()
}

fn head_verdict(status: u16) -> Option<bool> {
    match status {
        200..=299 => Some(true),
        404 | 410 => Some(false),
        _ => None,
    }
}

#[cfg(test)]
#[path = "s3_verifier_tests.rs"]
mod stored_tests;

#[cfg(test)]
mod tests {
    use super::head_verdict;

    #[test]
    fn only_a_real_404_marks_a_track_missing() {
        assert_eq!(head_verdict(200), Some(true));
        assert_eq!(head_verdict(404), Some(false));
        assert_eq!(head_verdict(410), Some(false));
        for busy in [429, 500, 502, 503, 504] {
            assert_eq!(head_verdict(busy), None, "{busy}");
        }
    }
}
