use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, Serializer};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::common::sc_ids::{EntityKind, EntityRef, require_ref};
use crate::error::{AppError, AppResult};

const SAME_PLAY_SECS: i64 = 60;
const CLOCK_SKEW_SECS: i64 = 120;
const PLAYED_AT_MAX_AGE_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ListeningHistory {
    pub id: Uuid,
    #[serde(rename = "soundcloudUserId")]
    pub soundcloud_user_id: String,
    #[serde(rename = "scTrackId")]
    pub sc_track_id: String,
    pub title: String,
    pub artist_name: String,
    pub artist_urn: Option<String>,
    pub artwork_url: Option<String>,
    pub duration: i32,
    #[serde(serialize_with = "ts_iso")]
    pub played_at: NaiveDateTime,
    #[serde(rename = "trackUrn")]
    pub track_urn: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordHistoryDto {
    #[serde(rename = "scTrackId")]
    pub sc_track_id: String,
    pub title: String,
    #[serde(rename = "artistName")]
    pub artist_name: String,
    #[serde(rename = "artistUrn")]
    pub artist_urn: Option<String>,
    #[serde(rename = "artworkUrl")]
    pub artwork_url: Option<String>,
    pub duration: i32,
    #[serde(rename = "playedAt", default)]
    pub played_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenedDto {
    pub listened_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordedHistory {
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub collection: Vec<ListeningHistory>,
    pub total: i64,
}

pub struct HistoryService {
    pub(super) pg: PgPool,
}

impl HistoryService {
    pub fn new(pg: PgPool) -> Arc<Self> {
        Arc::new(Self { pg })
    }

    pub async fn record(&self, sc_user_id: &str, data: &RecordHistoryDto) -> AppResult<Uuid> {
        let track = require_ref(EntityKind::Track, &data.sc_track_id)?;
        let track_urn = track.urn();
        let played_at = played_at(data.played_at, Utc::now());
        let same_play = Duration::seconds(SAME_PLAY_SECS);
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let mut tx = self.pg.begin().await?;
        sqlx::query_file_scalar!(
            "queries/history/service/lock_play.sql",
            sc_user_id,
            &track_urn
        )
        .fetch_one(&mut *tx)
        .await?;
        let same = sqlx::query_file_scalar!(
            "queries/history/service/find_same_play.sql",
            &variants,
            &[track_urn.clone(), track.sc_id()],
            played_at - same_play,
            played_at + same_play
        )
        .fetch_optional(&mut *tx)
        .await?;
        let id = match same {
            Some(id) => id,
            None => {
                sqlx::query_scalar(
                    "INSERT INTO listening_history \
                     (soundcloud_user_id, sc_track_id, title, artist_name, artist_urn, artwork_url, duration, played_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
                )
                .bind(sc_user_id)
                .bind(&track_urn)
                .bind(&data.title)
                .bind(&data.artist_name)
                .bind(&data.artist_urn)
                .bind(&data.artwork_url)
                .bind(data.duration)
                .bind(played_at)
                .fetch_one(&mut *tx)
                .await?
            }
        };
        tx.commit().await?;
        Ok(id)
    }

    pub async fn report_listened(
        &self,
        sc_user_id: &str,
        history_id: Uuid,
        listened_ms: i32,
    ) -> AppResult<()> {
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let stored = sqlx::query_file!(
            "queries/history/service/insert_listened.sql",
            history_id,
            &variants,
            listened_ms
        )
        .execute(&self.pg)
        .await?;
        if stored.rows_affected() == 0 {
            return Err(AppError::not_found("history entry not found"));
        }
        Ok(())
    }

    pub async fn find_all(
        &self,
        sc_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<HistoryPage> {
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let mut collection: Vec<ListeningHistory> = sqlx::query_file_as!(
            ListeningHistory,
            "queries/history/service/find_all_by_user.sql",
            &variants,
            limit,
            offset
        )
        .fetch_all(&self.pg)
        .await?;
        for entry in &mut collection {
            entry.track_urn = EntityRef::track(&entry.sc_track_id).map(EntityRef::urn);
        }
        let total =
            sqlx::query_file_scalar!("queries/history/service/count_by_user.sql", &variants)
                .fetch_one(&self.pg)
                .await?;
        Ok(HistoryPage { collection, total })
    }

    pub async fn clear(&self, sc_user_id: &str) -> AppResult<()> {
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let mut tx = self.pg.begin().await?;
        sqlx::query_file!(
            "queries/history/service/clear_listened_by_user.sql",
            &variants
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!("queries/history/service/clear_by_user.sql", &variants)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

fn played_at(sent: Option<DateTime<Utc>>, now: DateTime<Utc>) -> NaiveDateTime {
    let earliest = now - Duration::days(PLAYED_AT_MAX_AGE_DAYS);
    let latest = now + Duration::seconds(CLOCK_SKEW_SECS);
    sent.filter(|at| (earliest..=latest).contains(at))
        .unwrap_or(now)
        .min(now)
        .naive_utc()
}

pub fn ts_iso<S: Serializer>(dt: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(*dt, Utc);
    s.serialize_str(&utc.to_rfc3339_opts(SecondsFormat::Millis, true))
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
