use std::sync::Arc;

use backend_contracts::{JobKind, LyricsLookupPayload};
use catalog_normalize::{name_similarity, normalize_title};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::background_jobs::{BackgroundJob, BackgroundJobs};
use crate::error::{AppError, AppResult};

const LOOKUP_PRIORITY: i16 = 10;
const LOOKUP_MAX_ATTEMPTS: i16 = 8;
const MAX_OPERATIONAL_FAILURES: i32 = 8;

#[derive(Debug, Clone, Serialize)]
pub struct LyricsResponse {
    #[serde(rename = "scTrackId")]
    pub sc_track_id: Option<String>,
    #[serde(rename = "syncedLrc")]
    pub synced_lrc: Option<String>,
    #[serde(rename = "plainText")]
    pub plain_text: Option<String>,
    pub source: String,
    pub language: Option<String>,
    #[serde(rename = "languageConfidence")]
    pub language_confidence: Option<f32>,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct LyricsHints {
    pub title: String,
    pub artist: String,
    pub duration_sec: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
struct LyricsStatusRow {
    track_id: Uuid,
    synced_lrc: Option<String>,
    plain_text: Option<String>,
    source: Option<String>,
    language: Option<String>,
    language_confidence: Option<f32>,
    lookup_status: Option<String>,
    next_run_at: Option<DateTime<Utc>>,
    failure_streak: Option<i32>,
}

#[derive(Debug, Clone, FromRow)]
struct LyricsSearchRow {
    sc_track_id: String,
    metadata_artist: Option<String>,
    uploader_username: Option<String>,
    duration_ms: i32,
    synced_lrc: Option<String>,
    plain_text: Option<String>,
    source: String,
    language: Option<String>,
    language_confidence: Option<f32>,
}

pub struct LyricsService {
    pg: PgPool,
    background_jobs: Arc<BackgroundJobs>,
    reserve: bool,
}

impl LyricsService {
    pub fn new(pg: PgPool, background_jobs: Arc<BackgroundJobs>, reserve: bool) -> Arc<Self> {
        Arc::new(Self {
            pg,
            background_jobs,
            reserve,
        })
    }

    pub async fn ensure_lyrics(&self, sc_track_id: &str) -> AppResult<LyricsResponse> {
        let sc_track_id = canonical_track_id(sc_track_id)?;
        let status = self.status(&sc_track_id).await?;
        let Some(status) = status else {
            return Err(AppError::not_found("track not found"));
        };
        if status.source.is_some() {
            return Ok(found_response(&sc_track_id, status));
        }
        if is_fresh_negative(&status) {
            return Ok(none_response(Some(&sc_track_id)));
        }
        if is_operationally_blocked(&status) {
            return Err(AppError::service_unavailable(
                "lyrics lookup is temporarily unavailable",
            ));
        }
        if self.reserve {
            if status.lookup_status.is_some() {
                return Ok(pending_response(Some(&sc_track_id)));
            }
            return Err(AppError::service_unavailable(
                "lyrics lookup is unavailable on this replica",
            ));
        }

        let mut transaction = self.pg.begin().await?;
        sqlx::query_file!(
            "queries/lyrics/service/ensure_lookup_state.sql",
            &sc_track_id
        )
        .execute(&mut *transaction)
        .await?;
        let wake = sqlx::query_file!("queries/lyrics/service/request_lookup.sql", &sc_track_id)
            .fetch_optional(&mut *transaction)
            .await?;
        transaction.commit().await?;

        let Some(wake) = wake else {
            let status = self.status(&sc_track_id).await?;
            return match status {
                Some(status) if status.source.is_some() => Ok(found_response(&sc_track_id, status)),
                Some(status) if is_fresh_negative(&status) => Ok(none_response(Some(&sc_track_id))),
                Some(_) => Ok(pending_response(Some(&sc_track_id))),
                None => Err(AppError::not_found("track not found")),
            };
        };
        if wake.status == "not_found" && wake.next_run_at > Utc::now() {
            return Ok(none_response(Some(&sc_track_id)));
        }
        if wake.status == "retry" && wake.failure_streak >= MAX_OPERATIONAL_FAILURES {
            return Err(AppError::service_unavailable(
                "lyrics lookup is temporarily unavailable",
            ));
        }
        if wake.claim_job_id.is_none()
            && wake.wake_durable_at.is_none()
            && let (Some(message_id), Some(generation)) =
                (wake.wake_message_id, wake.wake_generation)
        {
            let job = BackgroundJob::coalescing(
                JobKind::LyricsLookup,
                &sc_track_id,
                LyricsLookupPayload {
                    sc_track_id: sc_track_id.clone(),
                },
            )?
            .with_id(message_id)
            .with_priority(LOOKUP_PRIORITY)
            .with_max_attempts(LOOKUP_MAX_ATTEMPTS)?
            .if_absent();
            match self.background_jobs.enqueue(&job).await {
                Ok(_) => {
                    sqlx::query_file!(
                        "queries/lyrics/service/mark_wake_durable.sql",
                        status.track_id,
                        generation,
                        message_id
                    )
                    .execute(&self.pg)
                    .await?;
                }
                Err(error) => {
                    tracing::warn!(track = %sc_track_id, %error, "lyrics wake publish deferred to sweep");
                }
            }
        }
        Ok(pending_response(Some(&sc_track_id)))
    }

    pub async fn search_lyrics(&self, hints: &LyricsHints) -> AppResult<LyricsResponse> {
        let title = hints.title.trim();
        let artist = hints.artist.trim();
        if title.is_empty() || artist.is_empty() {
            return Ok(none_response(None));
        }
        let normalized_title = normalize_title(title);
        if normalized_title.is_empty() {
            return Ok(none_response(None));
        }
        let candidates = sqlx::query_file_as!(
            LyricsSearchRow,
            "queries/lyrics/service/search_cache.sql",
            &normalized_title
        )
        .fetch_all(&self.pg)
        .await?;
        let selected = candidates
            .into_iter()
            .filter_map(|candidate| {
                let candidate_artist = candidate
                    .metadata_artist
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .or(candidate.uploader_username.as_deref())
                    .unwrap_or_default();
                let artist_score = name_similarity(artist, candidate_artist);
                if artist_score < 0.72
                    || !duration_matches(hints.duration_sec, candidate.duration_ms)
                {
                    return None;
                }
                Some((artist_score.to_bits(), candidate))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, candidate)| candidate);
        let Some(candidate) = selected else {
            return Ok(none_response(None));
        };
        Ok(LyricsResponse {
            sc_track_id: Some(candidate.sc_track_id),
            synced_lrc: candidate.synced_lrc,
            plain_text: candidate.plain_text,
            source: candidate.source,
            language: candidate.language,
            language_confidence: candidate.language_confidence,
            status: "found".to_owned(),
        })
    }

    async fn status(&self, sc_track_id: &str) -> AppResult<Option<LyricsStatusRow>> {
        Ok(sqlx::query_file_as!(
            LyricsStatusRow,
            "queries/lyrics/service/lyrics_status.sql",
            sc_track_id
        )
        .fetch_optional(&self.pg)
        .await?)
    }
}

fn canonical_track_id(value: &str) -> AppResult<String> {
    let value = value.trim();
    let value = value.strip_prefix("soundcloud:tracks:").unwrap_or(value);
    let id = value
        .parse::<u64>()
        .map_err(|_| AppError::bad_request("invalid SoundCloud track id"))?;
    if id == 0 || id.to_string() != value {
        return Err(AppError::bad_request("invalid SoundCloud track id"));
    }
    Ok(value.to_owned())
}

fn is_fresh_negative(status: &LyricsStatusRow) -> bool {
    status.lookup_status.as_deref() == Some("not_found")
        && status.next_run_at.is_some_and(|next| next > Utc::now())
}

fn is_operationally_blocked(status: &LyricsStatusRow) -> bool {
    status.lookup_status.as_deref() == Some("retry")
        && status.failure_streak.unwrap_or_default() >= MAX_OPERATIONAL_FAILURES
}

fn duration_matches(target_sec: Option<i64>, candidate_ms: i32) -> bool {
    let Some(target) = target_sec.filter(|value| *value > 0) else {
        return true;
    };
    if candidate_ms <= 0 {
        return true;
    }
    let candidate = (candidate_ms as f64 / 1000.0).round() as i64;
    let maximum = target.max(candidate) as f64;
    (target - candidate).abs() as f64 / maximum <= 0.25
}

fn found_response(sc_track_id: &str, row: LyricsStatusRow) -> LyricsResponse {
    LyricsResponse {
        sc_track_id: Some(sc_track_id.to_owned()),
        synced_lrc: row.synced_lrc,
        plain_text: row.plain_text,
        source: row.source.unwrap_or_else(|| "none".to_owned()),
        language: row.language,
        language_confidence: row.language_confidence,
        status: "found".to_owned(),
    }
}

fn pending_response(sc_track_id: Option<&str>) -> LyricsResponse {
    LyricsResponse {
        sc_track_id: sc_track_id.map(str::to_owned),
        synced_lrc: None,
        plain_text: None,
        source: "none".to_owned(),
        language: None,
        language_confidence: None,
        status: "pending".to_owned(),
    }
}

fn none_response(sc_track_id: Option<&str>) -> LyricsResponse {
    LyricsResponse {
        sc_track_id: sc_track_id.map(str::to_owned),
        synced_lrc: None,
        plain_text: None,
        source: "none".to_owned(),
        language: None,
        language_confidence: None,
        status: "none".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_id_boundary_is_strict() {
        assert_eq!(canonical_track_id("42").unwrap(), "42");
        assert_eq!(canonical_track_id("soundcloud:tracks:42").unwrap(), "42");
        for value in ["", "0", "042", "+42", "tracks:42", "foo:42"] {
            assert!(canonical_track_id(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn duration_filter_keeps_the_inclusive_boundary() {
        assert!(duration_matches(Some(100), 133_000));
        assert!(!duration_matches(Some(100), 134_000));
        assert!(duration_matches(None, 134_000));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_track_without_lyrics_or_lookup_state_still_reports_status(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ('42', 'soundcloud:tracks:42', 'Track', 'track', 120000)",
        )
        .execute(&pool)
        .await?;

        let row = sqlx::query_file_as!(
            LyricsStatusRow,
            "queries/lyrics/service/lyrics_status.sql",
            "42"
        )
        .fetch_optional(&pool)
        .await?
        .expect("status row");

        assert!(
            row.source.is_none()
                && row.synced_lrc.is_none()
                && row.plain_text.is_none()
                && row.language.is_none()
                && row.language_confidence.is_none(),
            "an unjoined lyrics cache must decode as absent, not fail: {row:?}"
        );
        Ok(())
    }

    async fn service(pool: &sqlx::PgPool, reserve: bool) -> anyhow::Result<Arc<LyricsService>> {
        let nats = crate::bus::nats::NatsService::connect(
            "nats://127.0.0.1:1",
            tokio_util::sync::CancellationToken::new(),
        )
        .await?;
        Ok(LyricsService::new(
            pool.clone(),
            BackgroundJobs::new(nats),
            reserve,
        ))
    }

    async fn seed_track(pool: &sqlx::PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ('42', 'soundcloud:tracks:42', 'Track', 'track', 120000)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn seed_cached(
        pool: &sqlx::PgPool,
        sc_track_id: &str,
        artist: &str,
        duration_ms: i32,
        text: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, urn, title, title_normalized, duration_ms, metadata_artist
             ) VALUES ($1, 'soundcloud:tracks:' || $1, 'Midnight Dreams', 'midnight dreams', $2, $3)",
        )
        .bind(sc_track_id)
        .bind(duration_ms)
        .bind(artist)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO lyrics_cache (sc_track_id, plain_text, source, language)
             VALUES ($1, $2, 'lrclib', 'en')",
        )
        .bind(sc_track_id)
        .bind(text)
        .execute(pool)
        .await?;
        Ok(())
    }

    fn hints(title: &str, artist: &str, duration_sec: Option<i64>) -> LyricsHints {
        LyricsHints {
            title: title.to_owned(),
            artist: artist.to_owned(),
            duration_sec,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_search_without_a_title_or_an_artist_asks_nothing_of_the_catalog(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        let lyrics = service(&pool, false).await?;

        for (title, artist) in [
            ("", "Boards of Canada"),
            ("Midnight Dreams", "  "),
            ("", ""),
        ] {
            let answer = lyrics.search_lyrics(&hints(title, artist, None)).await?;
            assert_eq!(answer.status, "none");
            assert!(answer.sc_track_id.is_none());
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn another_artist_with_the_same_title_is_not_served(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        let lyrics = service(&pool, false).await?;
        seed_cached(&pool, "100", "Boards of Canada", 240_000, "their words").await?;

        let answer = lyrics
            .search_lyrics(&hints("Midnight Dreams", "Aphex Twin", Some(240)))
            .await?;

        assert_eq!(
            answer.status, "none",
            "a title match alone must never hand over someone else's lyrics"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_same_artist_at_a_different_length_is_not_the_same_song(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        let lyrics = service(&pool, false).await?;
        seed_cached(&pool, "100", "Boards of Canada", 240_000, "their words").await?;

        let far = lyrics
            .search_lyrics(&hints("Midnight Dreams", "Boards of Canada", Some(60)))
            .await?;
        assert_eq!(far.status, "none", "a quarter is the whole tolerance");

        let near = lyrics
            .search_lyrics(&hints("Midnight Dreams", "Boards of Canada", Some(200)))
            .await?;
        assert_eq!(near.status, "found");
        assert_eq!(near.plain_text.as_deref(), Some("their words"));
        assert_eq!(near.sc_track_id.as_deref(), Some("100"));
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_closest_artist_wins_when_several_tracks_share_a_title(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        let lyrics = service(&pool, false).await?;
        seed_cached(&pool, "100", "Boards of Canada", 240_000, "exact words").await?;
        seed_cached(
            &pool,
            "101",
            "Boards of Canada Tribute",
            240_000,
            "tribute words",
        )
        .await?;

        let answer = lyrics
            .search_lyrics(&hints("Midnight Dreams", "Boards of Canada", Some(240)))
            .await?;

        assert_eq!(answer.sc_track_id.as_deref(), Some("100"));
        assert_eq!(answer.plain_text.as_deref(), Some("exact words"));
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_cache_row_cannot_even_be_written(pool: sqlx::PgPool) -> anyhow::Result<()> {
        seed_track(&pool).await?;

        let refused = sqlx::query(
            "INSERT INTO lyrics_cache (sc_track_id, plain_text, synced_lrc, source)
             VALUES ('42', '   ', NULL, 'lrclib')",
        )
        .execute(&pool)
        .await
        .expect_err("the schema must refuse lyrics that are only whitespace");

        assert!(
            refused.to_string().contains("lyrics_cache_text_present"),
            "the guard in search_cache.sql is a second line of defence: the first is this check \
             constraint, and it is the one that makes the state unreachable, saw {refused}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_track_is_not_found_rather_than_pending(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        let lyrics = service(&pool, false).await?;

        let error = lyrics.ensure_lyrics("42").await.unwrap_err();

        assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn stored_lyrics_are_served_without_touching_the_queue(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        seed_track(&pool).await?;
        sqlx::query(
            "INSERT INTO lyrics_cache (sc_track_id, source, plain_text, synced_lrc, language)
             VALUES ('42', 'lrclib', 'a line', '[00:01.00] a line', 'en')",
        )
        .execute(&pool)
        .await?;
        let lyrics = service(&pool, false).await?;

        let response = lyrics.ensure_lyrics("soundcloud:tracks:42").await?;

        assert_eq!(response.status, "found");
        assert_eq!(response.source, "lrclib");
        assert_eq!(response.plain_text.as_deref(), Some("a line"));
        let queued: i64 =
            sqlx::query_scalar("SELECT count(*) FROM background_jobs WHERE kind = 'lyrics.lookup'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(queued, 0);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_reserve_replica_answers_pending_without_touching_the_lookup(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        seed_track(&pool).await?;
        let before: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT updated_at FROM lyrics_lookup_state")
                .fetch_optional(&pool)
                .await?;
        let lyrics = service(&pool, true).await?;

        let response = lyrics.ensure_lyrics("42").await?;

        assert_eq!(response.status, "pending");
        let after: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT updated_at FROM lyrics_lookup_state")
                .fetch_optional(&pool)
                .await?;
        assert_eq!(
            before, after,
            "a reserve replica must not rewrite the lookup state"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_reserve_replica_without_lookup_state_refuses_instead_of_pretending(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        seed_track(&pool).await?;
        sqlx::query("DELETE FROM lyrics_lookup_state")
            .execute(&pool)
            .await?;
        let lyrics = service(&pool, true).await?;

        let error = lyrics.ensure_lyrics("42").await.unwrap_err();

        assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_missing_lookup_becomes_a_durable_request_and_a_pending_answer(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        seed_track(&pool).await?;
        let lyrics = service(&pool, false).await?;

        let response = lyrics.ensure_lyrics("42").await?;

        assert_eq!(response.status, "pending");
        assert_eq!(response.sc_track_id.as_deref(), Some("42"));
        let (priority, wake): (i16, Option<uuid::Uuid>) = sqlx::query_as(
            "SELECT priority, wake_message_id FROM lyrics_lookup_state WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(priority, 0, "a user request must take the top priority");
        assert!(wake.is_some(), "the wake marker must survive in PostgreSQL");
        Ok(())
    }
}
