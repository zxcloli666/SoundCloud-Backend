use sqlx::PgPool;

use crate::config::PlaylistReconcileConfig;
use crate::queue::{JobError, JobResult};

pub struct PlaylistLegacyHandler {
    pool: PgPool,
    reconcile: PlaylistReconcileConfig,
}

impl PlaylistLegacyHandler {
    pub fn new(pool: PgPool, reconcile: PlaylistReconcileConfig) -> Self {
        Self { pool, reconcile }
    }

    pub async fn drain(&self) -> JobResult {
        let batch = self.reconcile.legacy_drain_batch;
        let drained =
            sqlx::query_file_scalar!("queries/playlist_legacy/drain_safe_intents.sql", batch)
                .fetch_all(&self.pool)
                .await
                .map_err(JobError::retryable)?;

        if !drained.is_empty() {
            let mut playlists = drained;
            playlists.sort();
            playlists.dedup();
            sqlx::query_file!(
                "queries/playlist_legacy/wake_drained_playlists.sql",
                &playlists
            )
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            tracing::info!(
                resolved = playlists.len(),
                "playlist legacy intents drained"
            );
        }

        sqlx::query_file!(
            "queries/playlist_legacy/wake_unclassified_playlists.sql",
            batch
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }
}
