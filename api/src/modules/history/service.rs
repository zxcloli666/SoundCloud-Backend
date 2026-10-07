use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, Serializer};
use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::common::sc_ids::{EntityKind, EntityRef, require_ref};
use crate::error::AppResult;

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
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub collection: Vec<ListeningHistory>,
    pub total: i64,
}

pub struct HistoryService {
    pg: PgPool,
}

impl HistoryService {
    pub fn new(pg: PgPool) -> Arc<Self> {
        Arc::new(Self { pg })
    }

    pub async fn record(&self, sc_user_id: &str, data: &RecordHistoryDto) -> AppResult<()> {
        let track = require_ref(EntityKind::Track, &data.sc_track_id)?;
        let track_urn = track.urn();
        let cutoff = chrono::Utc::now().naive_utc() - Duration::seconds(60);
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let recent = sqlx::query_file_scalar!(
            "queries/history/service/find_recent_play.sql",
            &variants,
            &[track_urn.clone(), track.sc_id()],
            cutoff
        )
        .fetch_optional(&self.pg)
        .await?;
        if recent.is_some() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO listening_history \
             (soundcloud_user_id, sc_track_id, title, artist_name, artist_urn, artwork_url, duration) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(sc_user_id)
        .bind(&track_urn)
        .bind(&data.title)
        .bind(&data.artist_name)
        .bind(&data.artist_urn)
        .bind(&data.artwork_url)
        .bind(data.duration)
        .execute(&self.pg)
        .await?;
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
        sqlx::query_file!("queries/history/service/clear_by_user.sql", &variants)
            .execute(&self.pg)
            .await?;
        Ok(())
    }
}

pub fn ts_iso<S: Serializer>(dt: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(*dt, Utc);
    s.serialize_str(&utc.to_rfc3339_opts(SecondsFormat::Millis, true))
}
