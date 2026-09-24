mod entities;
mod error;
mod genius;
mod identity;
mod musicbrainz;
mod socials;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use catalog_match::AccountRole;
use catalog_sources::{GeniusService, MbClient};
use chrono::{DateTime, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::CrawlConfig;
use crate::queue::{JobError, JobRepository, JobResult};

use super::catalog_read::PublicCatalogReader;
use super::lyrics::wake;
use super::wanted::WantedHandler;

use self::error::{CrawlError, CrawlResult};
use self::socials::SocialLink;

const BACKOFF_BASE: Duration = Duration::from_secs(60 * 60);
const BACKOFF_CAP: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CRAWL_ARTIST_DEADLINE: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Genius,
    MusicBrainz,
}

struct Claimed {
    id: Uuid,
    mb_artist_id: Option<String>,
    genius_artist_id: Option<String>,
    sc_user_id: Option<String>,
    mb_crawl_offset: i32,
    genius_crawl_offset: i32,
    crawl_fail_count: i16,
}

pub struct CrawlHandler {
    pool: PgPool,
    queue: JobRepository,
    mb: Arc<MbClient>,
    genius: Arc<GeniusService>,
    reader: Arc<PublicCatalogReader>,
    wanted: Arc<WantedHandler>,
    config: CrawlConfig,
}

impl CrawlHandler {
    pub fn new(
        pool: PgPool,
        mb: Arc<MbClient>,
        genius: Arc<GeniusService>,
        reader: Arc<PublicCatalogReader>,
        wanted: Arc<WantedHandler>,
        config: CrawlConfig,
    ) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "catalog-crawl".to_owned()),
            pool,
            mb,
            genius,
            reader,
            wanted,
            config,
        }
    }

    pub async fn crawl_genius_lane(&self) -> JobResult {
        self.resolve_pending_identities().await?;
        self.run_lane(
            Lane::Genius,
            self.config.genius_batch,
            self.config.genius_concurrency,
        )
        .await
    }

    pub async fn crawl_musicbrainz_lane(&self) -> JobResult {
        self.run_lane(
            Lane::MusicBrainz,
            self.config.mb_batch,
            self.config.mb_concurrency,
        )
        .await
    }

    pub async fn crawl_artist(&self, artist_id: Uuid) -> JobResult {
        let claimed = sqlx::query_file!("queries/crawl/claim_single_artist.sql", artist_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        let Some(row) = claimed else {
            return Ok(());
        };
        let artist = Claimed {
            id: row.id,
            mb_artist_id: row.mb_artist_id,
            genius_artist_id: row.genius_artist_id,
            sc_user_id: row.sc_user_id,
            mb_crawl_offset: row.mb_crawl_offset,
            genius_crawl_offset: row.genius_crawl_offset,
            crawl_fail_count: 0,
        };
        match self.crawl_one(&artist, None).await {
            Ok(()) => {
                self.resolve_wanted_after_crawl(&artist).await;
                Ok(())
            }
            Err(error) => Err(JobError::retryable(error)),
        }
    }

    async fn resolve_pending_identities(&self) -> JobResult {
        let pending = sqlx::query_file!(
            "queries/crawl/claim_identity_lane.sql",
            self.config.lease_seconds as f64,
            self.config.identity_batch
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if pending.is_empty() {
            return Ok(());
        }

        let examined = pending.len();
        let mut matched = 0usize;
        let mut queue = pending.into_iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < self.config.genius_concurrency
                && let Some(artist) = queue.next()
            {
                running.push(async move {
                    let result = identity::resolve_genius_id(
                        &self.pool,
                        &self.genius,
                        artist.id,
                        &artist.name,
                        self.config.recrawl_days as f64,
                    )
                    .await;
                    (artist.id, result)
                });
            }
            let Some((artist_id, result)) = running.next().await else {
                break;
            };
            match result {
                Ok(true) => matched += 1,
                Ok(false) => {}
                Err(error) => {
                    warn!(artist = %artist_id, %error, "genius identity lookup failed");
                    self.defer_identity(artist_id).await;
                }
            }
        }
        info!(examined, matched, "verified artists looked up on genius");
        Ok(())
    }

    async fn defer_identity(&self, artist_id: Uuid) {
        let deferred = sqlx::query_file!(
            "queries/crawl/defer_identity_lookup.sql",
            artist_id,
            self.config.recrawl_days as f64
        )
        .execute(&self.pool)
        .await;
        if let Err(error) = deferred {
            warn!(%artist_id, %error, "releasing the identity lease failed");
        }
    }

    async fn run_lane(&self, lane: Lane, batch: i64, concurrency: usize) -> JobResult {
        let artists = self.claim_lane(lane, batch).await?;
        if artists.is_empty() {
            return Ok(());
        }

        let mut crawled = 0usize;
        let mut failed = 0usize;
        let mut running = FuturesUnordered::new();
        let mut queue = artists.into_iter();

        loop {
            while running.len() < concurrency.max(1)
                && let Some(artist) = queue.next()
            {
                running.push(self.settle(lane, artist));
            }
            let Some(outcome) = running.next().await else {
                break;
            };
            match outcome {
                Ok(true) => crawled += 1,
                Ok(false) => failed += 1,
                Err(error) => {
                    failed += 1;
                    warn!(%error, "crawl bookkeeping failed");
                }
            }
        }

        info!(
            lane = lane.as_str(),
            crawled, failed, "artist crawl finished"
        );
        Ok(())
    }

    async fn settle(&self, lane: Lane, artist: Claimed) -> CrawlResult<bool> {
        let result =
            match tokio::time::timeout(CRAWL_ARTIST_DEADLINE, self.crawl_one(&artist, Some(lane)))
                .await
            {
                Ok(result) => result,
                Err(_) => Err(CrawlError::Deadline),
            };
        match result {
            Ok(()) => {
                self.mark_success(lane, artist.id).await?;
                self.resolve_wanted_after_crawl(&artist).await;
                Ok(true)
            }
            Err(error) => {
                debug!(artist = %artist.id, %error, "artist crawl failed");
                self.mark_failure(lane, &artist).await?;
                Ok(false)
            }
        }
    }

    async fn crawl_one(&self, artist: &Claimed, lane: Option<Lane>) -> CrawlResult {
        let crawl_musicbrainz = lane != Some(Lane::Genius);
        let crawl_genius = lane != Some(Lane::MusicBrainz);
        let mut links: Vec<SocialLink> = Vec::new();
        let mut country: Option<String> = None;
        let mut avatar_url: Option<String> = None;
        let mut bio: Option<String> = None;

        if crawl_musicbrainz && let Some(mb_artist_id) = artist.mb_artist_id.as_deref() {
            match self.mb.lookup_artist(mb_artist_id).await {
                Ok(Some(details)) => {
                    country = details.country.filter(|value| !value.is_empty());
                    bio = details.disambiguation.filter(|value| !value.is_empty());
                    links.extend(details.urls.iter().filter_map(socials::from_musicbrainz));
                }
                Ok(None) => debug!(artist = %artist.id, mb_artist_id, "musicbrainz artist is gone"),
                Err(error) => return Err(error.into()),
            }
        }

        let genius_artist_id = artist
            .genius_artist_id
            .as_deref()
            .and_then(|id| id.parse::<i64>().ok());
        if crawl_genius
            && let Some(genius_artist_id) = genius_artist_id
            && let Some(details) = self.genius.lookup_artist(genius_artist_id).await
        {
            avatar_url = details.avatar_url.clone().filter(|url| !url.is_empty());
            links.extend(socials::from_genius(&details));
        }

        if let Some(sc_user_id) = artist.sc_user_id.as_deref() {
            links.extend(socials::from_soundcloud_profile(&self.reader, sc_user_id).await);
        }

        links.sort_by(|left, right| left.url.cmp(&right.url));
        links.dedup_by(|left, right| left.url == right.url);
        socials::store(&self.pool, artist.id, &links).await?;
        self.update_metadata(artist.id, country, avatar_url, bio)
            .await?;
        self.adopt_linked_accounts(artist.id, &links).await;

        if crawl_musicbrainz && let Some(mb_artist_id) = artist.mb_artist_id.as_deref() {
            let next_offset = musicbrainz::discover_tracks(
                &self.pool,
                &self.mb,
                artist.id,
                mb_artist_id,
                artist.mb_crawl_offset as u32,
            )
            .await?;
            sqlx::query_file!(
                "queries/crawl/set_mb_crawl_offset.sql",
                artist.id,
                next_offset as i32
            )
            .execute(&self.pool)
            .await?;
        }

        if crawl_genius && let Some(genius_artist_id) = genius_artist_id {
            let songs = genius::discover_songs(
                &self.pool,
                &self.genius,
                artist.id,
                genius_artist_id,
                artist.genius_crawl_offset as u32,
            )
            .await;
            if let Ok(next_offset) = &songs {
                sqlx::query_file!(
                    "queries/crawl/set_genius_crawl_offset.sql",
                    artist.id,
                    *next_offset as i32
                )
                .execute(&self.pool)
                .await?;
            }
            let albums =
                genius::discover_albums(&self.pool, &self.genius, artist.id, genius_artist_id)
                    .await;
            propagate_unless_payload_failure(artist.id, "genius songs", songs.map(|_| ()))?;
            propagate_unless_payload_failure(artist.id, "genius albums", albums)?;
            self.enqueue_artist_lyrics(artist.id).await;
        }
        Ok(())
    }

    async fn adopt_linked_accounts(&self, artist_id: Uuid, links: &[SocialLink]) {
        for link in links {
            if link.kind != "soundcloud" || !catalog_match::is_soundcloud_url(&link.url) {
                continue;
            }
            let resolved = match self.reader.resolve_url(&link.url).await {
                Ok(resolved) => resolved,
                Err(error) => {
                    debug!(url = %link.url, %error, "resolving the artist soundcloud url failed");
                    continue;
                }
            };
            let Some(sc_user_id) = catalog_match::extract_sc_user_id(&resolved) else {
                continue;
            };
            let stored = catalog_match::upsert_account(
                &self.pool,
                artist_id,
                &sc_user_id,
                AccountRole::Main,
                "mb_resolve",
                false,
            )
            .await;
            if let Err(error) = stored {
                if catalog_match::claims_another_artist(&error) {
                    warn!(%artist_id, sc_user_id, "soundcloud account already belongs to another artist");
                    continue;
                }
                debug!(%artist_id, sc_user_id, %error, "storing the artist soundcloud account failed");
            }
        }
    }

    async fn update_metadata(
        &self,
        artist_id: Uuid,
        country: Option<String>,
        avatar_url: Option<String>,
        bio: Option<String>,
    ) -> CrawlResult {
        if country.is_none() && avatar_url.is_none() && bio.is_none() {
            return Ok(());
        }
        sqlx::query_file!(
            "queries/crawl/update_artist_metadata.sql",
            artist_id,
            country.as_deref(),
            avatar_url.as_deref(),
            bio.as_deref()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn enqueue_artist_lyrics(&self, artist_id: Uuid) {
        let tracks = sqlx::query_file_scalar!(
            "queries/crawl/lyrics_wakes_for_artist.sql",
            artist_id,
            self.config.post_crawl_wanted_max
        )
        .fetch_all(&self.pool)
        .await;
        let tracks = match tracks {
            Ok(tracks) => tracks,
            Err(error) => {
                debug!(%artist_id, %error, "loading artist lyrics wakes failed");
                return;
            }
        };
        for sc_track_id in tracks {
            if let Err(error) = wake::enqueue(&self.pool, &self.queue, &sc_track_id).await {
                debug!(%artist_id, %sc_track_id, %error, "lyrics wake deferred to sweep");
            }
        }
    }

    async fn resolve_wanted_after_crawl(&self, artist: &Claimed) {
        if artist.sc_user_id.is_none() {
            return;
        }
        if let Err(error) = self
            .wanted
            .resolve_for_artist(artist.id, self.config.post_crawl_wanted_max)
            .await
        {
            debug!(artist = %artist.id, %error, "post-crawl wanted resolve failed");
        }
    }

    async fn claim_lane(&self, lane: Lane, batch: i64) -> JobResult<Vec<Claimed>> {
        let lease = self.config.lease_seconds as f64;
        let claimed = match lane {
            Lane::Genius => sqlx::query_file!("queries/crawl/claim_genius_lane.sql", lease, batch)
                .fetch_all(&self.pool)
                .await
                .map_err(JobError::retryable)?
                .into_iter()
                .map(|row| Claimed {
                    id: row.id,
                    mb_artist_id: row.mb_artist_id,
                    genius_artist_id: row.genius_artist_id,
                    sc_user_id: row.sc_user_id,
                    mb_crawl_offset: row.mb_crawl_offset,
                    genius_crawl_offset: row.genius_crawl_offset,
                    crawl_fail_count: row.crawl_fail_count,
                })
                .collect(),
            Lane::MusicBrainz => sqlx::query_file!("queries/crawl/claim_mb_lane.sql", lease, batch)
                .fetch_all(&self.pool)
                .await
                .map_err(JobError::retryable)?
                .into_iter()
                .map(|row| Claimed {
                    id: row.id,
                    mb_artist_id: row.mb_artist_id,
                    genius_artist_id: row.genius_artist_id,
                    sc_user_id: row.sc_user_id,
                    mb_crawl_offset: row.mb_crawl_offset,
                    genius_crawl_offset: row.genius_crawl_offset,
                    crawl_fail_count: row.crawl_fail_count,
                })
                .collect(),
        };
        Ok(claimed)
    }

    async fn mark_success(&self, lane: Lane, artist_id: Uuid) -> CrawlResult {
        let recrawl_days = self.config.recrawl_days as f64;
        match lane {
            Lane::Genius => {
                sqlx::query_file!(
                    "queries/crawl/lane_success_genius.sql",
                    artist_id,
                    recrawl_days
                )
                .execute(&self.pool)
                .await?;
            }
            Lane::MusicBrainz => {
                sqlx::query_file!("queries/crawl/lane_success_mb.sql", artist_id, recrawl_days)
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(())
    }

    async fn mark_failure(&self, lane: Lane, artist: &Claimed) -> CrawlResult {
        let fail_count = artist.crawl_fail_count.saturating_add(1);
        if fail_count >= self.config.max_fails {
            match lane {
                Lane::Genius => {
                    sqlx::query_file!("queries/crawl/lane_dead_genius.sql", artist.id, fail_count)
                        .execute(&self.pool)
                        .await?;
                }
                Lane::MusicBrainz => {
                    sqlx::query_file!("queries/crawl/lane_dead_mb.sql", artist.id, fail_count)
                        .execute(&self.pool)
                        .await?;
                }
            }
            return Ok(());
        }

        let next_run_at = next_run_after(fail_count);
        match lane {
            Lane::Genius => {
                sqlx::query_file!(
                    "queries/crawl/lane_backoff_genius.sql",
                    artist.id,
                    fail_count,
                    next_run_at
                )
                .execute(&self.pool)
                .await?;
            }
            Lane::MusicBrainz => {
                sqlx::query_file!(
                    "queries/crawl/lane_backoff_mb.sql",
                    artist.id,
                    fail_count,
                    next_run_at
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }
}

impl Lane {
    fn as_str(self) -> &'static str {
        match self {
            Self::Genius => "genius",
            Self::MusicBrainz => "musicbrainz",
        }
    }
}

fn propagate_unless_payload_failure(
    artist_id: Uuid,
    stage: &'static str,
    outcome: CrawlResult,
) -> CrawlResult {
    match outcome {
        Ok(()) => Ok(()),
        Err(error) if error.is_payload_failure() => {
            warn!(artist = %artist_id, stage, %error, "external payload changed shape, artist kept alive");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn next_run_after(fail_count: i16) -> DateTime<Utc> {
    let shift = fail_count.clamp(0, 16) as u32;
    let seconds = BACKOFF_BASE
        .as_secs()
        .saturating_mul(1u64 << shift)
        .min(BACKOFF_CAP.as_secs());
    Utc::now() + chrono::Duration::seconds(seconds as i64)
}
