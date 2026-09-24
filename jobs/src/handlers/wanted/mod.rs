mod account_scan;
mod ai_matcher;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use catalog_ingest::{ScTrackFields, TrackPriority};
use catalog_match::{evaluate_sc_candidate, sc_track_id_from_urn};
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::Value;
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::bus::Bus;
use crate::config::WantedConfig;
use crate::queue::{JobError, JobRepository, JobResult};

use super::catalog_read::PublicCatalogReader;
use super::lyrics::wake;

use self::ai_matcher::{AiMatcherClient, AiUnavailable, MatchCandidate, MatchTarget};

const SEARCH_LIMIT: i64 = 10;
const WANTED_DEADLINE: Duration = Duration::from_secs(120);
const AI_RETRY_DELAY: Duration = Duration::from_secs(30 * 60);

pub const LINK_THRESHOLD: f32 = 0.7;
pub const BORDERLINE_FLOOR: f32 = 0.45;

#[derive(Debug, Clone)]
pub struct WantedTrack {
    pub id: Uuid,
    pub title: String,
    pub artist_name: String,
    pub duration_ms: Option<i32>,
    pub isrc: Option<String>,
    pub primary_artist_id: Option<Uuid>,
}

pub struct WantedHandler {
    pool: PgPool,
    queue: JobRepository,
    reader: Arc<PublicCatalogReader>,
    ai: Option<AiMatcherClient>,
    config: WantedConfig,
}

impl WantedHandler {
    pub fn new(
        pool: PgPool,
        reader: Arc<PublicCatalogReader>,
        bus: Bus,
        config: WantedConfig,
    ) -> Self {
        let ai = config.ai_enabled.then(|| {
            AiMatcherClient::new(
                bus,
                pool.clone(),
                config.ai_timeout_ms,
                config.ai_daily_budget,
            )
        });
        Self {
            queue: JobRepository::new(pool.clone(), "wanted".to_owned()),
            pool,
            reader,
            ai,
            config,
        }
    }

    pub async fn resolve_due(&self) -> JobResult {
        let claimed = sqlx::query_file_scalar!(
            "queries/wanted/claim_batch.sql",
            self.config.lease_seconds as f64,
            self.config.batch
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if claimed.is_empty() {
            return Ok(());
        }

        let tracks = sqlx::query_file_as!(WantedTrack, "queries/wanted/fetch_by_ids.sql", &claimed)
            .fetch_all(&self.pool)
            .await
            .map_err(JobError::retryable)?;

        let deferred = self.resolve_batch(tracks).await;
        let (deferred, settled): (Vec<Uuid>, Vec<Uuid>) =
            claimed.into_iter().partition(|id| deferred.contains(id));
        self.finalize(&settled).await?;
        self.defer(&deferred).await
    }

    pub async fn resolve_for_artist(&self, artist_id: Uuid, limit: i64) -> JobResult {
        let tracks = sqlx::query_file_as!(
            WantedTrack,
            "queries/wanted/fetch_for_artist.sql",
            artist_id,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        self.resolve_batch(tracks).await;
        Ok(())
    }

    async fn resolve_batch(&self, tracks: Vec<WantedTrack>) -> HashSet<Uuid> {
        let mut deferred = HashSet::new();
        if tracks.is_empty() {
            return deferred;
        }
        info!(batch = tracks.len(), "wanted resolve batch started");

        let linked = self.scan_identity_accounts(&tracks).await;
        let pending: Vec<&WantedTrack> = tracks
            .iter()
            .filter(|track| !linked.contains(&track.id))
            .collect();

        let mut running = FuturesUnordered::new();
        let mut queue = pending.into_iter();
        let concurrency = self.config.search_concurrency.max(1);
        loop {
            while running.len() < concurrency
                && let Some(track) = queue.next()
            {
                running.push(async move {
                    let track_id = track.id;
                    let outcome =
                        tokio::time::timeout(WANTED_DEADLINE, self.search_and_link(track)).await;
                    (track_id, outcome)
                });
            }
            let Some((track_id, outcome)) = running.next().await else {
                break;
            };
            match outcome {
                Ok(Resolution::Deferred) => {
                    deferred.insert(track_id);
                }
                Ok(_) => {}
                Err(_) => warn!(wanted = %track_id, "wanted resolve exceeded its deadline"),
            }
        }
        deferred
    }

    async fn scan_identity_accounts(&self, tracks: &[WantedTrack]) -> HashSet<Uuid> {
        let mut by_artist: HashMap<Uuid, Vec<&WantedTrack>> = HashMap::new();
        for track in tracks {
            if let Some(artist_id) = track.primary_artist_id {
                by_artist.entry(artist_id).or_default().push(track);
            }
        }

        let mut linked = HashSet::new();
        let mut pending = by_artist.into_iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < self.config.search_concurrency
                && let Some((artist_id, group)) = pending.next()
            {
                running.push(async move {
                    let result = match tokio::time::timeout(
                        WANTED_DEADLINE,
                        self.scan_artist_uploads(artist_id, &group),
                    )
                    .await
                    {
                        Ok(Ok(ids)) => Ok(ids),
                        Ok(Err(error)) => Err(error.to_string()),
                        Err(_) => Err("deadline exceeded".to_owned()),
                    };
                    (artist_id, result)
                });
            }
            let Some((artist_id, result)) = running.next().await else {
                break;
            };
            match result {
                Ok(ids) => linked.extend(ids),
                Err(error) => warn!(%artist_id, %error, "identity account scan failed"),
            }
        }
        linked
    }

    async fn search_and_link(&self, track: &WantedTrack) -> Resolution {
        match self.resolve_one(track).await {
            Ok(Resolution::Unmatched) => {
                let touched = sqlx::query_file!("queries/wanted/touch_updated_at.sql", track.id)
                    .execute(&self.pool)
                    .await;
                if let Err(error) = touched {
                    debug!(wanted = %track.id, %error, "touching the wanted track failed");
                }
                Resolution::Unmatched
            }
            Ok(resolution) => resolution,
            Err(error) => {
                warn!(wanted = %track.id, %error, "wanted resolve failed");
                Resolution::Unmatched
            }
        }
    }

    async fn resolve_one(&self, track: &WantedTrack) -> Result<Resolution, sqlx::Error> {
        if let Some(local) = self.local_match(track).await? {
            let linked = catalog_match::link_wanted_to_sc(&self.pool, track.id, &local).await?;
            if linked {
                info!(wanted = %track.id, sc_track_id = %local, "wanted linked to an indexed track");
                return Ok(Resolution::Linked);
            }
        }

        let candidates = self.search(track).await;
        if candidates.is_empty() {
            return Ok(Resolution::Unmatched);
        }
        let triaged = triage(&candidates, track, LINK_THRESHOLD);

        if let Some((index, score)) = triaged.best
            && let Some(candidate) = candidates.get(index)
        {
            return self
                .ingest_and_link(track, candidate, score, "sc_search")
                .await
                .map(Resolution::from_linked);
        }
        let chosen = match self.ask_ai(track, &candidates, &triaged.borderline).await {
            Ok(Some(chosen)) => chosen,
            Ok(None) => return Ok(Resolution::Unmatched),
            Err(AiUnavailable) => {
                info!(wanted = %track.id, "ai matcher is unavailable; the attempt is not counted");
                return Ok(Resolution::Deferred);
            }
        };
        let Some(candidate) = candidates.get(chosen.0) else {
            return Ok(Resolution::Unmatched);
        };
        self.ingest_and_link(track, candidate, chosen.1, "sc_search+ai")
            .await
            .map(Resolution::from_linked)
    }

    async fn local_match(&self, track: &WantedTrack) -> Result<Option<String>, sqlx::Error> {
        let Some(artist_id) = track.primary_artist_id else {
            return Ok(None);
        };
        let found =
            catalog_match::best_indexed_for_artist_title(&self.pool, artist_id, &track.title)
                .await?;
        Ok(found.map(|matched| matched.sc_track_id))
    }

    async fn search(&self, track: &WantedTrack) -> Vec<Value> {
        let queries = if track.artist_name.is_empty() {
            vec![track.title.clone()]
        } else {
            vec![
                format!("{} {}", track.artist_name, track.title),
                track.title.clone(),
            ]
        };

        let mut found: Vec<Value> = Vec::new();
        for query in queries {
            match self.reader.search_tracks(&query, SEARCH_LIMIT).await {
                Ok(items) if !items.is_empty() => {
                    found.extend(items);
                    if found.len() >= SEARCH_LIMIT as usize {
                        break;
                    }
                }
                Ok(_) => {}
                Err(error) => debug!(wanted = %track.id, %error, "soundcloud search failed"),
            }
        }
        found
    }

    async fn ask_ai(
        &self,
        track: &WantedTrack,
        candidates: &[Value],
        borderline: &[usize],
    ) -> Result<Option<(usize, f32)>, AiUnavailable> {
        let Some(ai) = self.ai.as_ref() else {
            return Ok(None);
        };
        let offered = offered_candidates(candidates, borderline);
        if offered.is_empty() {
            return Ok(None);
        }
        let offers: Vec<MatchCandidate> = offered
            .iter()
            .enumerate()
            .map(|(offer_id, (_, candidate))| MatchCandidate::from_sc(offer_id as u32, candidate))
            .collect();
        let picked = ai
            .pick(
                MatchTarget {
                    artist: &track.artist_name,
                    title: &track.title,
                },
                &offers,
            )
            .await?;
        Ok(picked.and_then(|picked| {
            let (index, _) = offered.get(picked.candidate_id as usize)?;
            Some((*index, picked.confidence))
        }))
    }

    async fn ingest_and_link(
        &self,
        track: &WantedTrack,
        candidate: &Value,
        score: f32,
        via: &'static str,
    ) -> Result<bool, sqlx::Error> {
        let Some(sc_track_id) = candidate
            .get("urn")
            .and_then(Value::as_str)
            .and_then(sc_track_id_from_urn)
        else {
            return Ok(false);
        };
        let Some(fields) = ScTrackFields::from_sc(candidate) else {
            return Ok(false);
        };
        let result = catalog_ingest::upsert_from_sc(
            &self.pool,
            &fields,
            TrackPriority::Discovery,
            TrackPriority::Discovery,
            catalog_ingest::Observation::UNVERIFIED,
        )
        .await?;
        if result.was_new
            && let Err(error) = wake::enqueue(&self.pool, &self.queue, &sc_track_id).await
        {
            warn!(%sc_track_id, %error, "lyrics wake deferred to sweep");
        }

        let linked = catalog_match::link_wanted_to_sc(&self.pool, track.id, &sc_track_id).await?;
        if linked {
            info!(wanted = %track.id, sc_track_id, score, via, "wanted linked");
        }
        Ok(linked)
    }

    async fn defer(&self, deferred: &[Uuid]) -> JobResult {
        if deferred.is_empty() {
            return Ok(());
        }
        sqlx::query_file!(
            "queries/wanted/finalize_deferred.sql",
            deferred,
            AI_RETRY_DELAY.as_secs_f64()
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/wanted/finalize_clear_locks.sql", deferred)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn finalize(&self, claimed: &[Uuid]) -> JobResult {
        if claimed.is_empty() {
            return Ok(());
        }
        sqlx::query_file!(
            "queries/wanted/finalize_backoff.sql",
            claimed,
            self.config.max_attempts
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/wanted/finalize_clear_locks.sql", claimed)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(())
    }
}

fn offered_candidates<'a>(
    candidates: &'a [Value],
    borderline: &[usize],
) -> Vec<(usize, &'a Value)> {
    borderline
        .iter()
        .filter_map(|&index| Some((index, candidates.get(index)?)))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    Linked,
    Unmatched,
    Deferred,
}

impl Resolution {
    fn from_linked(linked: bool) -> Self {
        if linked {
            Self::Linked
        } else {
            Self::Unmatched
        }
    }
}

pub struct Triage {
    pub best: Option<(usize, f32)>,
    pub borderline: Vec<usize>,
}

pub fn triage(candidates: &[Value], track: &WantedTrack, link_threshold: f32) -> Triage {
    let mut best: Option<(usize, f32)> = None;
    let mut borderline = Vec::new();

    for (index, candidate) in candidates.iter().enumerate() {
        let score = evaluate_sc_candidate(
            candidate,
            &track.title,
            &track.artist_name,
            track.isrc.as_deref(),
            track.duration_ms,
        )
        .score();

        if score >= link_threshold {
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((index, score));
            }
        } else if score >= BORDERLINE_FLOOR {
            borderline.push(index);
        }
    }
    Triage { best, borderline }
}
