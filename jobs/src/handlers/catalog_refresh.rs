use std::sync::Arc;
use std::time::Duration;

use backend_contracts::{CatalogEntity, CatalogRefreshPayload};
use serde_json::Value;
use sqlx::PgPool;

use crate::config::JobsConfig;
use crate::queue::{JobError, JobResult, LeasedJob};

use super::catalog_read::PublicCatalogReader;
use super::catalog_remote::{CatalogRemote, public_error};

const FETCH_DEADLINE: Duration = Duration::from_secs(45);
const GEO_REGIONS: i32 = 3;

pub struct CatalogRefreshHandler {
    writer: super::catalog_refresh_writer::CatalogWriter,
    public: Arc<PublicCatalogReader>,
    remote: CatalogRemote,
}

impl CatalogRefreshHandler {
    pub fn new(
        pool: PgPool,
        public: Arc<PublicCatalogReader>,
        config: &JobsConfig,
    ) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            writer: super::catalog_refresh_writer::CatalogWriter::new(
                pool.clone(),
                config.durations.max_track_duration_ms,
            ),
            remote: CatalogRemote::new(pool, config)?,
            public,
        })
    }

    pub async fn refresh(&self, job: &LeasedJob, payload: CatalogRefreshPayload) -> JobResult {
        if !payload.is_valid() {
            return Err(JobError::permanent(anyhow::anyhow!(
                "invalid catalog refresh resource"
            )));
        }
        if self.writer.is_deleted(&payload).await? {
            return Ok(());
        }
        let observation = self.writer.begin_observation().await?;
        let mut value = tokio::time::timeout(FETCH_DEADLINE, self.fetch(&payload))
            .await
            .map_err(|_| {
                JobError::retryable(anyhow::anyhow!("catalog refresh deadline exceeded"))
            })??;
        sc_transport::normalize_v2_to_v1(&mut value);
        validate_entity(&payload, &value)?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "urn".into(),
                Value::String(payload.entity.urn(&payload.sc_id)),
            );
        }
        self.writer
            .persist(job, &payload, &value, observation)
            .await
    }

    async fn fetch(&self, payload: &CatalogRefreshPayload) -> JobResult<Value> {
        let path = payload.entity.path(&payload.sc_id);
        if payload.entity == CatalogEntity::WebProfiles {
            return self.remote.public_get(&path).await;
        }
        match payload.owner_id.as_deref() {
            Some(owner) => self.remote.owner_get(owner, &path).await,
            None => self.public_get_across_regions(&path).await,
        }
    }

    async fn public_get_across_regions(&self, path: &str) -> JobResult<Value> {
        let mut last = None;
        for region in 0..GEO_REGIONS {
            match self.public.get_json_from_region(path, region).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let absent = matches!(&error, sc_transport::ScError::Api { status: 404, .. });
                    if !absent {
                        return Err(public_error(error));
                    }
                    tracing::debug!(
                        path,
                        region,
                        "catalog entity is absent in this region, rotating"
                    );
                    last = Some(error);
                }
            }
        }
        Err(public_error(last.unwrap_or_else(|| {
            sc_transport::ScError::invalid("catalog entity was never requested")
        })))
    }
}

pub(super) fn validate_entity(payload: &CatalogRefreshPayload, value: &Value) -> JobResult {
    if payload.entity == CatalogEntity::WebProfiles {
        return super::catalog_web_profiles::validate(value);
    }
    let id = value
        .get("urn")
        .and_then(Value::as_str)
        .map(catalog_ingest::extract_sc_id)
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| id.to_string())
        });
    let title_field = match payload.entity {
        CatalogEntity::Track | CatalogEntity::Playlist => "title",
        CatalogEntity::User | CatalogEntity::Profile | CatalogEntity::WebProfiles => "username",
    };
    if id.as_deref() != Some(&payload.sc_id)
        || value
            .get("urn")
            .and_then(Value::as_str)
            .is_some_and(|urn| urn != payload.entity.urn(&payload.sc_id))
        || value
            .get("id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id.to_string() != payload.sc_id)
        || value
            .get(title_field)
            .and_then(Value::as_str)
            .is_none_or(|title| title.trim().is_empty())
    {
        return Err(JobError::retryable(anyhow::anyhow!(
            "catalog response identity is invalid"
        )));
    }
    if matches!(
        payload.entity,
        CatalogEntity::Track | CatalogEntity::Playlist
    ) {
        let sharing = value.get("sharing").and_then(Value::as_str);
        if !matches!(sharing, Some("public" | "private")) {
            return Err(JobError::retryable(anyhow::anyhow!(
                "catalog sharing is missing or invalid"
            )));
        }
        if sharing == Some("public") {
            return Ok(());
        }
        let owner = value
            .pointer("/user/urn")
            .and_then(Value::as_str)
            .map(catalog_ingest::extract_sc_id)
            .map(str::to_owned);
        if payload.owner_id.is_none() || owner != payload.owner_id {
            return Err(JobError::permanent(anyhow::anyhow!(
                "catalog response is private to another account"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::catalog_remote::{connection_error, owner_error};
    use crate::handlers::playlist_observe::PlaylistReadError;
    use crate::handlers::sync_queue::ConnectionError;

    #[test]
    fn cooldowns_and_reauthorization_postpone_without_spending_attempts() {
        let failures = [
            connection_error(ConnectionError::ReauthorizationRequired),
            connection_error(ConnectionError::TemporarilyUnavailable {
                retry_after_seconds: 600,
            }),
            public_error(sc_transport::ScError::Api {
                status: 429,
                body: Value::Null,
                retry_after_sec: Some(700),
            }),
            owner_error(PlaylistReadError::Api {
                status: wreq::StatusCode::UNAUTHORIZED,
                body: Value::Null,
                retry_after_seconds: None,
            }),
        ];
        for (failure, seconds) in failures.into_iter().zip([900, 600, 700, 300]) {
            assert!(
                matches!(failure, JobError::Postponed { delay, .. } if delay == Duration::from_secs(seconds))
            );
        }
    }

    #[test]
    fn refresh_rejects_a_private_response_from_a_public_or_different_owner_context() {
        let value = serde_json::json!({ "urn": "soundcloud:tracks:42", "title": "test", "sharing": "private", "user": { "urn": "soundcloud:users:17" } });
        let mut payload = CatalogRefreshPayload {
            entity: CatalogEntity::Track,
            sc_id: "42".into(),
            owner_id: None,
        };
        assert!(validate_entity(&payload, &value).is_err());
        payload.owner_id = Some("18".into());
        assert!(validate_entity(&payload, &value).is_err());
        payload.owner_id = Some("17".into());
        assert!(validate_entity(&payload, &value).is_ok());
    }

    #[test]
    fn refresh_rejects_an_unrelated_or_incomplete_entity() {
        let payload = CatalogRefreshPayload {
            entity: CatalogEntity::User,
            sc_id: "17".into(),
            owner_id: None,
        };
        assert!(
            validate_entity(&payload, &serde_json::json!({"id": 18, "username": "test"})).is_err()
        );
        assert!(validate_entity(&payload, &serde_json::json!({"id": 17})).is_err());
        assert!(
            validate_entity(
                &payload,
                &serde_json::json!({"urn": "soundcloud:tracks:17", "username": "test"})
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod geo_tests {
    use super::*;

    fn absent() -> sc_transport::ScError {
        sc_transport::ScError::Api {
            status: 404,
            body: serde_json::Value::Null,
            retry_after_sec: None,
        }
    }

    #[test]
    fn a_region_that_hides_a_track_is_not_the_final_answer() {
        let regions: i32 = GEO_REGIONS;
        assert!(regions > 1, "one region cannot prove absence");
        assert!(matches!(
            public_error(absent()),
            crate::queue::JobError::Permanent(_)
        ));
    }

    #[test]
    fn a_blocked_region_answers_differently_from_an_absent_entity() {
        let blocked = sc_transport::ScError::Api {
            status: 403,
            body: serde_json::Value::Null,
            retry_after_sec: None,
        };
        assert!(matches!(
            public_error(blocked),
            crate::queue::JobError::Postponed { .. }
        ));
    }
}
