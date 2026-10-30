#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;

use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use super::{PgError, PgPool};

const QUALITY: &str = "single";

const INSERT_CDN_TRACK: &str = r#"INSERT INTO cdn_tracks (id, track_urn, quality, cdn_path, status, created_at, updated_at, last_accessed_at)
   VALUES (public.scd_uuid('cdn_tracks', $1::text, $2::text), $1, $2, $3, $4, NOW(), NOW(), NOW())
   ON CONFLICT (track_urn, quality) DO UPDATE SET status = $4, cdn_path = $3, updated_at = NOW()
   RETURNING id"#;

const SET_CDN_STATUS: &str =
    r#"UPDATE cdn_tracks SET status = $2, updated_at = NOW() WHERE id = $1::text::uuid"#;

#[derive(Debug)]
pub struct CdnTrackRecord {
    pub id: String,
}

impl PgPool {
    pub async fn find_cached_track(
        &self,
        track_urn: &str,
    ) -> Result<Option<CdnTrackRecord>, PgError> {
        let client = self.connection().await?;
        let row = client
            .query_opt(
                r#"SELECT id
                   FROM cdn_tracks
                   WHERE track_urn = $1 AND quality = 'single' AND status = 'ok'"#,
                &[&track_urn],
            )
            .await?;
        Ok(row.map(|row| CdnTrackRecord {
            id: row.get::<_, Uuid>(0).to_string(),
        }))
    }

    pub async fn update_last_accessed(&self, id: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client
            .execute(
                r#"UPDATE cdn_tracks SET last_accessed_at = NOW() WHERE id = $1::text::uuid"#,
                &[&id],
            )
            .await?;
        Ok(())
    }

    pub async fn insert_cdn_track(
        &self,
        track_urn: &str,
        cdn_path: &str,
        status: &str,
    ) -> Result<String, PgError> {
        let client = self.connection().await?;
        let params: [&(dyn ToSql + Sync); 4] = [&track_urn, &QUALITY, &cdn_path, &status];
        let row = match client.query_one(INSERT_CDN_TRACK, &params).await {
            Err(e) if e.code() == Some(&SqlState::UNIQUE_VIOLATION) => {
                client.query_one(INSERT_CDN_TRACK, &params).await?
            }
            row => row?,
        };
        Ok(row.get::<_, Uuid>(0).to_string())
    }

    pub async fn update_cdn_track_status(&self, id: &str, status: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client.execute(SET_CDN_STATUS, &[&id, &status]).await?;
        Ok(())
    }

    pub async fn mark_storage_lost(&self, cdn_id: &str, track_urn: &str) -> Result<bool, PgError> {
        let mut client = self.connection().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(SET_CDN_STATUS, &[&cdn_id, &"error"])
            .await?;
        let lost = wt::TrackStorageLost {
            urn: track_urn.to_owned(),
            at: chrono::Utc::now(),
        };
        let marked = wt::tokio_pg::call(&*transaction, &lost).await?;
        transaction.commit().await?;
        Ok(marked)
    }
}
