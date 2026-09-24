mod mirror;
mod page;
mod state;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::CatalogCollectionPayload;
use sqlx::PgPool;

use crate::config::JobsConfig;
use crate::queue::{JobError, JobResult, LeasedJob};

use super::catalog_read::PublicCatalogReader;
use super::catalog_remote::{CatalogRemote, postpone, public_error};

pub struct CatalogCollectionHandler {
    pool: PgPool,
    public: Arc<PublicCatalogReader>,
    remote: CatalogRemote,
    writer: writer::CollectionWriter,
}

impl CatalogCollectionHandler {
    pub fn new(
        pool: PgPool,
        public: Arc<PublicCatalogReader>,
        config: &JobsConfig,
    ) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            remote: CatalogRemote::new(pool.clone(), config)?,
            writer: writer::CollectionWriter::new(
                pool.clone(),
                config.durations.max_track_duration_ms,
            ),
            pool,
            public,
        })
    }

    pub async fn refresh(&self, job: &LeasedJob, payload: CatalogCollectionPayload) -> JobResult {
        if !payload.is_valid() {
            return Err(JobError::permanent(anyhow::anyhow!(
                "invalid collection resource"
            )));
        }
        let Some(snapshot) = state::begin(&self.pool, job, &payload).await? else {
            return Ok(());
        };
        if snapshot.complete {
            return Ok(());
        }
        let observation = catalog_ingest::Observation::begin(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        let page = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(&payload, snapshot.next_cursor.as_deref()),
        )
        .await
        .map_err(|_| JobError::retryable(anyhow::anyhow!("collection page deadline exceeded")))?;
        let page = match page {
            Err(JobError::Permanent(error)) => return Err(postpone(1800, error)),
            result => result?,
        };
        let more = page.next.is_some();
        self.writer
            .persist(job, &payload, &snapshot, page, observation)
            .await?;
        if more {
            Err(postpone(1, anyhow::anyhow!("collection has another page")))
        } else {
            Ok(())
        }
    }

    async fn fetch(
        &self,
        payload: &CatalogCollectionPayload,
        cursor: Option<&str>,
    ) -> JobResult<page::Page> {
        let public_apiv2 = !payload.owner && payload.collection.public_apiv2();
        let (apiv2, path) = match cursor {
            Some(cursor) => page::target(payload, cursor)?,
            None => (
                public_apiv2,
                format!(
                    "{}?limit={}&linked_partitioning=true",
                    payload.path(public_apiv2),
                    page::PAGE_SIZE
                ),
            ),
        };
        let path = page::request_path(payload, &path)?;
        if payload.owner {
            return page::parse(
                payload,
                self.remote.owner_get(&payload.subject_id, &path).await?,
                false,
            );
        }
        if !apiv2 {
            return page::parse(payload, self.remote.public_get(&path).await?, false);
        }
        match self.public.get_json(&path).await {
            Ok(value) => page::parse(payload, value, true),
            Err(error)
                if cursor.is_none()
                    && !matches!(error, sc_transport::ScError::Api { status: 404, .. }) =>
            {
                let path = page::request_path(
                    payload,
                    &format!(
                        "{}?limit={}&linked_partitioning=true",
                        payload.path(false),
                        page::PAGE_SIZE
                    ),
                )?;
                page::parse(payload, self.remote.public_get(&path).await?, false)
            }
            Err(error) => Err(public_error(error)),
        }
    }
}

#[cfg(test)]
mod tests;
