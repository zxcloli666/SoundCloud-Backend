use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use sqlx::PgPool;
use tracing::debug;
use wreq::Client;

use crate::error::AppResult;

const HEAD_CONCURRENCY: usize = 16;
const HEAD_TIMEOUT: Duration = Duration::from_secs(3);

pub struct S3VerifierService {
    http: Client,
    storage_url: String,
    pg: PgPool,
}

impl S3VerifierService {
    pub fn new(http: Client, storage_url: String, pg: PgPool) -> Arc<Self> {
        let trimmed = storage_url.trim_end_matches('/').to_string();
        Arc::new(Self {
            http,
            storage_url: trimmed,
            pg,
        })
    }

    pub async fn find_missing(&self, sc_track_ids: &[String]) -> AppResult<HashSet<String>> {
        let mut missing: HashSet<String> = HashSet::new();
        if sc_track_ids.is_empty() || self.storage_url.is_empty() {
            return Ok(missing);
        }

        let ttl_cutoff = Utc::now() - chrono::Duration::hours(24);

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
                Some((Some(verified), m)) if m.map(|x| x <= *verified).unwrap_or(true) => {
                    continue;
                }
                Some((_, Some(miss))) if *miss > ttl_cutoff => {
                    missing.insert(id.clone());
                }
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

        if !ok_ids.is_empty() {
            sqlx::query_file!("queries/recommendations/s3_verifier/mark_ok.sql", &ok_ids)
                .execute(&self.pg)
                .await?;
        }
        if !miss_ids.is_empty() {
            sqlx::query_file!(
                "queries/recommendations/s3_verifier/mark_missing.sql",
                &miss_ids
            )
            .execute(&self.pg)
            .await?;
            debug!(
                misses = miss_ids.len(),
                oks = ok_ids.len(),
                "S3 verify result"
            );
        }
        Ok(missing)
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

fn head_verdict(status: u16) -> Option<bool> {
    match status {
        200..=299 => Some(true),
        404 | 410 => Some(false),
        _ => None,
    }
}

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
