mod mirror;
mod page;
mod state;
mod writer;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use backend_contracts::{
    CATALOG_REFRESH_BACKGROUND_PRIORITY, CatalogCollectionPayload, CatalogEntity, CollectionFailure,
};
use chrono::Utc;
use sqlx::PgPool;

use crate::config::JobsConfig;
use crate::queue::{JobError, JobResult, LeasedJob};

use super::catalog_read::PublicCatalogReader;
use super::catalog_remote::{
    CatalogRemote, owner_is_signed_out, postpone, public_error, refused_as_forbidden,
    waits_for_application,
};

const APIV2_DEADLINE: Duration = Duration::from_secs(25);
const APIV2_TRIES: i32 = 4;
const NOT_FOUND_PATIENCE: chrono::Duration = chrono::Duration::minutes(25);
const FORBIDDEN_PATIENCE: chrono::Duration = chrono::Duration::hours(6);

enum Fetched {
    Page(page::Page),
    OwnerApiNeeded,
    Apiv2Again,
}

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
                config.audience_max_items,
                config.followers_reconcile_max_rows,
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
        let step = Mutex::new("apiv2");
        let page = tokio::time::timeout(
            Duration::from_secs(45),
            self.fetch(job, &payload, snapshot.next_cursor.as_deref(), &step),
        )
        .await
        .map_err(|_| {
            let step = *step.lock().unwrap_or_else(PoisonError::into_inner);
            tracing::warn!(
                job = %job.id,
                collection = payload.collection.as_str(),
                owner = payload.owner,
                step,
                "collection page deadline exceeded"
            );
            JobError::retryable(anyhow::anyhow!("collection page deadline exceeded"))
        })?;
        let page = match page {
            Err(error) if payload.owner && owner_is_signed_out(&error) => {
                return self.finish_without_owner(job, &payload, &snapshot).await;
            }
            Err(error) => return Err(give_up_or_wait(&snapshot, error)),
            Ok(Fetched::Page(page)) => page,
            Ok(Fetched::OwnerApiNeeded) => {
                state::abandon_apiv2(&self.pool, job, &payload).await?;
                return Err(postpone(
                    1,
                    anyhow::anyhow!("collection restarts on the owner API"),
                ));
            }
            Ok(Fetched::Apiv2Again) => {
                state::start_over(&self.pool, job, &payload).await?;
                return Err(postpone(1, anyhow::anyhow!("collection restarts on apiv2")));
            }
        };
        let more = self
            .writer
            .persist(job, &payload, &snapshot, page, observation)
            .await?;
        if more {
            Err(another_page(&payload))
        } else {
            Ok(())
        }
    }

    async fn finish_without_owner(
        &self,
        job: &LeasedJob,
        payload: &CatalogCollectionPayload,
        snapshot: &state::Snapshot,
    ) -> JobResult {
        let owner_api_start = page::owner_api_start(payload)?;
        if snapshot.next_cursor.as_deref() == Some(&owner_api_start) {
            state::finish_unverified(&self.pool, job, payload).await?;
        }
        Ok(())
    }

    async fn fetch(
        &self,
        job: &LeasedJob,
        payload: &CatalogCollectionPayload,
        cursor: Option<&str>,
        step: &Mutex<&'static str>,
    ) -> JobResult<Fetched> {
        let enter = |name: &'static str| {
            *step.lock().unwrap_or_else(PoisonError::into_inner) = name;
        };
        let (apiv2, path) = match cursor {
            Some(cursor) => page::target(payload, cursor)?,
            None => (true, page::first_path(payload, true)),
        };
        let path = page::request_path(payload, &path)?;
        if !apiv2 {
            enter("apiv1");
            return match self.apiv1(job, payload, &path).await {
                Err(error) if !payload.owner && waits_for_application(&error) => {
                    Ok(Fetched::Apiv2Again)
                }
                page => page.map(Fetched::Page),
            };
        }
        enter("apiv2");
        let failure = match self.apiv2(payload, &path).await {
            Ok(value) => match page::parse(payload, value, true) {
                Ok(page) => return Ok(Fetched::Page(page)),
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
            Err(error) => {
                let unreachable = !matches!(error, sc_transport::ScError::Api { status: 404, .. });
                let failure = public_error(error);
                if unreachable && job.attempts < APIV2_TRIES {
                    return Err(failure);
                }
                failure
            }
        };
        if payload.owner {
            tracing::debug!(
                error = %failure,
                collection = payload.collection.as_str(),
                "apiv2 could not answer an owner collection, reading the owner API"
            );
            if cursor.is_some() {
                return Ok(Fetched::OwnerApiNeeded);
            }
        }
        let path = page::request_path(payload, &page::first_path(payload, false))?;
        enter("apiv1 after apiv2");
        self.apiv1(job, payload, &path).await.map(Fetched::Page)
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

    async fn apiv1(
        &self,
        job: &LeasedJob,
        payload: &CatalogCollectionPayload,
        path: &str,
    ) -> JobResult<page::Page> {
        let value = if payload.owner {
            self.remote
                .owner_get(job, &payload.subject_id, path)
                .await?
        } else {
            self.remote.public_get(job, path).await?
        };
        page::parse(payload, value, false)
    }
}

fn another_page(payload: &CatalogCollectionPayload) -> JobError {
    let wait = postpone(1, anyhow::anyhow!("collection has another page"));
    if payload.owner {
        wait.leaving_read_lane()
    } else {
        wait.stepping_down_to(CATALOG_REFRESH_BACKGROUND_PRIORITY)
    }
}

fn give_up_or_wait(snapshot: &state::Snapshot, error: JobError) -> JobError {
    let on_first_page = snapshot.next_cursor.is_none();
    let age = Utc::now().signed_duration_since(snapshot.started_at);
    match error {
        JobError::Permanent(error) if on_first_page && age >= NOT_FOUND_PATIENCE => {
            JobError::settled(CollectionFailure::NotFound, error)
        }
        JobError::Permanent(error) => postpone(1800, error),
        error if on_first_page && refused_as_forbidden(&error) && age >= FORBIDDEN_PATIENCE => {
            JobError::settled(CollectionFailure::Forbidden, error)
        }
        error => error,
    }
}

#[cfg(test)]
mod tests;
