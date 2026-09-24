use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use catalog_ingest::{ScTrackFields, TrackPriority, extract_sc_id};
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::Value;
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::AccountWalkConfig;
use crate::queue::{JobError, JobRepository, JobResult};

use super::catalog_read::PublicCatalogReader;
use super::lyrics::wake;

const PER_ACCOUNT_PAGES: usize = 5;
const PAGE_SIZE: i64 = 100;
const MAX_GEO_ATTEMPTS: usize = 3;
const WALK_DEADLINE: Duration = Duration::from_secs(180);

pub struct AccountWalkHandler {
    pool: PgPool,
    queue: JobRepository,
    reader: Arc<PublicCatalogReader>,
    config: AccountWalkConfig,
}

impl AccountWalkHandler {
    pub fn new(pool: PgPool, reader: Arc<PublicCatalogReader>, config: AccountWalkConfig) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "account-walk".to_owned()),
            pool,
            reader,
            config,
        }
    }

    pub async fn run(&self) -> JobResult {
        let artists = sqlx::query_file_scalar!(
            "queries/account_walk/claim.sql",
            self.config.walk_interval_days,
            self.config.lease_seconds,
            self.config.batch
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if artists.is_empty() {
            return Ok(());
        }

        let mut ingested = 0usize;
        let mut pending = artists.into_iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < self.config.concurrency
                && let Some(artist_id) = pending.next()
            {
                running.push(self.settle_walk(artist_id));
            }
            let Some(result) = running.next().await else {
                break;
            };
            ingested += result?;
        }
        if ingested > 0 {
            info!(ingested, "artist account walk ingested uploads");
        }
        Ok(())
    }

    async fn settle_walk(&self, artist_id: Uuid) -> JobResult<usize> {
        let result = match tokio::time::timeout(WALK_DEADLINE, self.walk(artist_id)).await {
            Ok(result) => result,
            Err(_) => Err(WalkError::Deadline),
        };
        match result {
            Ok(count) => {
                sqlx::query_file!("queries/account_walk/clear_lock_success.sql", artist_id)
                    .execute(&self.pool)
                    .await
                    .map_err(JobError::retryable)?;
                Ok(count)
            }
            Err(error) => {
                warn!(artist = %artist_id, %error, "account walk failed");
                sqlx::query_file!("queries/account_walk/clear_lock_failure.sql", artist_id)
                    .execute(&self.pool)
                    .await
                    .map_err(JobError::retryable)?;
                Ok(0)
            }
        }
    }

    async fn walk(&self, artist_id: Uuid) -> Result<usize, WalkError> {
        let accounts: Vec<String> =
            sqlx::query_file_scalar!("queries/account_walk/list_accounts.sql", artist_id)
                .fetch_all(&self.pool)
                .await?;
        if accounts.is_empty() {
            return Ok(0);
        }

        let mut ingested = 0usize;
        let mut avatar: Option<String> = None;
        for sc_user_id in accounts {
            let observation = catalog_ingest::Observation::begin(&self.pool).await?;
            for track in self.account_uploads(&sc_user_id).await? {
                if avatar.is_none() {
                    avatar = avatar_of(&track);
                }
                let Some(fields) = ScTrackFields::from_sc(&track) else {
                    continue;
                };
                let result = catalog_ingest::upsert_from_sc(
                    &self.pool,
                    &fields,
                    TrackPriority::Discovery,
                    TrackPriority::Discovery,
                    observation,
                )
                .await?;
                if result.was_new {
                    wake::enqueue(&self.pool, &self.queue, &fields.sc_track_id)
                        .await
                        .map_err(|error| WalkError::Lyrics(error.to_string()))?;
                }
                ingested += 1;
            }
        }

        if let Some(avatar) = avatar {
            sqlx::query_file!("queries/account_walk/set_avatar.sql", artist_id, &avatar)
                .execute(&self.pool)
                .await?;
        }
        Ok(ingested)
    }

    async fn account_uploads(&self, sc_user_id: &str) -> Result<Vec<Value>, WalkError> {
        let expected = self.expected_track_count(sc_user_id).await?;
        let mut by_id: HashMap<String, Value> = HashMap::new();

        for attempt in 0..MAX_GEO_ATTEMPTS {
            let (tracks, exhausted) = self.uploads_page_walk(sc_user_id, attempt as i32).await?;
            for track in tracks {
                if let Some(id) = sc_id_of(&track) {
                    by_id.entry(id).or_insert(track);
                }
            }
            let collected = by_id.len() as i64;
            let Some(expected) = expected else { break };
            if collected >= expected || !exhausted {
                break;
            }
            if attempt + 1 == MAX_GEO_ATTEMPTS {
                warn!(
                    sc_user_id,
                    expected,
                    collected,
                    gap = expected - collected,
                    "account listing stays geo-incomplete after every rotated region"
                );
                break;
            }
            debug!(
                sc_user_id,
                expected, collected, attempt, "short listing, rotating the relay region"
            );
        }
        Ok(by_id.into_values().collect())
    }

    async fn uploads_page_walk(
        &self,
        sc_user_id: &str,
        region_rotation: i32,
    ) -> Result<(Vec<Value>, bool), WalkError> {
        let path = format!("/users/{sc_user_id}/tracks");
        let mut collected: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut exhausted = false;

        for _ in 0..PER_ACCOUNT_PAGES {
            let page = self
                .reader
                .list_page(&path, cursor.as_deref(), PAGE_SIZE, region_rotation)
                .await?;
            if page.items.is_empty() {
                exhausted = true;
                break;
            }
            collected.extend(page.items);
            match page.next_href {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => {
                    exhausted = true;
                    break;
                }
            }
        }
        Ok((collected, exhausted))
    }

    async fn expected_track_count(&self, sc_user_id: &str) -> Result<Option<i64>, WalkError> {
        let user = self.reader.user_by_id(sc_user_id).await?;
        Ok(user.get("track_count").and_then(Value::as_i64))
    }
}

#[derive(Debug, thiserror::Error)]
enum WalkError {
    #[error("account walk database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("account walk SoundCloud request failed: {0}")]
    SoundCloud(#[from] sc_transport::ScError),
    #[error("account walk lyrics enqueue failed: {0}")]
    Lyrics(String),
    #[error("account walk exceeded its deadline")]
    Deadline,
}

fn sc_id_of(track: &Value) -> Option<String> {
    track
        .get("urn")
        .and_then(Value::as_str)
        .map(|urn| extract_sc_id(urn).to_owned())
        .filter(|id| !id.is_empty())
}

fn avatar_of(track: &Value) -> Option<String> {
    track
        .get("user")
        .and_then(|user| user.get("avatar_url"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(|url| url.replace("-large.", "-t500x500."))
}
