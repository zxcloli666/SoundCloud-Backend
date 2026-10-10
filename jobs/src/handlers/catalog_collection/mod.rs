mod mirror;
mod page;
mod state;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::{CatalogCollectionPayload, CatalogEntity};
use sqlx::PgPool;

use crate::config::JobsConfig;
use crate::queue::{JobError, JobResult, LeasedJob};

use super::catalog_read::PublicCatalogReader;
use super::catalog_remote::{CatalogRemote, postpone, public_error};

const APIV2_DEADLINE: Duration = Duration::from_secs(25);

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
        let Some(page) = page else {
            state::abandon_apiv2(&self.pool, job, &payload).await?;
            return Err(postpone(
                1,
                anyhow::anyhow!("collection restarts on the owner API"),
            ));
        };
        let more = self
            .writer
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
    ) -> JobResult<Option<page::Page>> {
        let (apiv2, path) = match cursor {
            Some(cursor) => page::target(payload, cursor)?,
            None => {
                let apiv2 = payload.apiv2_first();
                (apiv2, page::first_path(payload, apiv2))
            }
        };
        let path = page::request_path(payload, &path)?;
        if !apiv2 {
            return self.apiv1(payload, &path).await.map(Some);
        }
        let failure = match self.apiv2(payload, &path).await {
            Ok(value) => match page::parse(payload, value, true) {
                Ok(page) => return Ok(Some(page)),
                Err(error) if !payload.owner => return Err(error),
                Err(error) => error,
            },
            Err(error)
                if !payload.owner
                    && (cursor.is_some()
                        || matches!(error, sc_transport::ScError::Api { status: 404, .. })) =>
            {
                return Err(public_error(error));
            }
            Err(error) => public_error(error),
        };
        if payload.owner {
            tracing::debug!(
                error = %failure,
                collection = payload.collection.as_str(),
                "apiv2 could not answer an owner collection, reading the owner API"
            );
            if cursor.is_some() {
                return Ok(None);
            }
        }
        let path = page::request_path(payload, &page::first_path(payload, false))?;
        self.apiv1(payload, &path).await.map(Some)
    }

    async fn apiv2(
        &self,
        payload: &CatalogCollectionPayload,
        path: &str,
    ) -> sc_transport::ScResult<serde_json::Value> {
        let read = async {
            if payload.collection.subject() == CatalogEntity::User {
                self.public
                    .user_collection_json(
                        &payload.subject_id,
                        payload.collection.path_segment(true),
                        path,
                    )
                    .await
            } else {
                self.public.get_json(path).await
            }
        };
        tokio::time::timeout(APIV2_DEADLINE, read)
            .await
            .unwrap_or_else(|_| {
                Err(sc_transport::ScError::Unreachable(
                    "apiv2 collection read timed out".to_owned(),
                ))
            })
    }

    async fn apiv1(&self, payload: &CatalogCollectionPayload, path: &str) -> JobResult<page::Page> {
        let value = if payload.owner {
            self.remote.owner_get(&payload.subject_id, path).await?
        } else {
            self.remote.public_get(path).await?
        };
        page::parse(payload, value, false)
    }
}

#[cfg(test)]
mod tests;
