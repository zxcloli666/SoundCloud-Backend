mod classification;
mod client;
mod fingerprint;
mod model;
mod reduce;
mod remote;
mod repository;
mod urn;

#[cfg(test)]
#[path = "reduce_tests.rs"]
mod reduce_tests;

use std::time::Duration;

use backend_contracts::{JobKind, PlaylistObservePayload, Versioned};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;
use wreq::StatusCode;

use crate::config::{JobsConfig, PlaylistReconcileConfig};
use crate::queue::{JobError, JobRepository, JobResult, NewJob};

pub(super) use self::client::{PlaylistReadClient, PlaylistReadError};
use self::remote::{PlaylistObserveError, PlaylistReader};
use self::repository::{
    CaptureResult, FailureObservation, ObservationCapture, PersistResult,
    PlaylistObserveRepository, RepositoryError,
};
use self::urn::PlaylistUrn;
use super::lyrics::wake;
use super::sync_queue::{ConnectionError, ConnectionManager, TokenRefreshClient};

const REAUTHORIZATION_RETRY: Duration = Duration::from_secs(15 * 60);
const RATE_LIMIT_RETRY: Duration = Duration::from_secs(5 * 60);
const TRANSPORT_RETRY: Duration = Duration::from_secs(2 * 60);
const MALFORMED_RETRY: Duration = Duration::from_secs(5 * 60);
const FORBIDDEN_RETRY: Duration = Duration::from_secs(30 * 60);
const MAX_REMOTE_RETRY: Duration = Duration::from_secs(24 * 60 * 60);
const OBSERVE_MAX_ATTEMPTS: i16 = 8;

pub struct PlaylistObserveHandler {
    pool: PgPool,
    queue: JobRepository,
    repository: PlaylistObserveRepository,
    connections: ConnectionManager,
    token_client: TokenRefreshClient,
    reader: PlaylistReader,
    reconcile: PlaylistReconcileConfig,
}

impl PlaylistObserveHandler {
    pub fn new(config: &JobsConfig, pool: PgPool) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            queue: JobRepository::new(pool.clone(), "playlist-observe".to_owned()),
            repository: PlaylistObserveRepository::new(
                pool.clone(),
                config.playlist_reconcile.membership_remote_apply,
            ),
            connections: ConnectionManager::new(pool.clone()),
            pool,
            token_client: TokenRefreshClient::new(&config.oauth)?,
            reader: PlaylistReader::new(PlaylistReadClient::new(&config.sync_queue)?),
            reconcile: config.playlist_reconcile,
        })
    }

    pub async fn sweep_due(&self) -> JobResult {
        let due = self
            .repository
            .claim_due(
                self.reconcile.sweep_batch,
                self.reconcile.sweep_owner_share,
                self.reconcile.claim_seconds,
            )
            .await
            .map_err(repository_job_error)?;
        for playlist_urn in due {
            let payload = serde_json::to_value(Versioned::V1(PlaylistObservePayload {
                playlist_urn: playlist_urn.clone(),
            }))
            .map_err(JobError::permanent)?;
            let job = NewJob {
                id: Uuid::now_v7(),
                kind: JobKind::PlaylistObserveShadow,
                dedup_key: Some(playlist_urn.clone()),
                payload,
                priority: 0,
                max_attempts: OBSERVE_MAX_ATTEMPTS,
                available_at: Utc::now(),
            };
            if let Err(error) = self.queue.enqueue_if_absent(&job).await {
                tracing::warn!(playlist = %playlist_urn, %error, "playlist observation enqueue deferred to the next sweep");
            }
        }
        Ok(())
    }

    pub async fn observe(
        &self,
        job_id: Uuid,
        job_generation: i64,
        payload: PlaylistObservePayload,
    ) -> JobResult {
        let urn = PlaylistUrn::parse(&payload.playlist_urn).map_err(JobError::permanent)?;
        let captured = self
            .repository
            .capture(&urn, job_id, job_generation)
            .await
            .map_err(repository_job_error)?;
        if captured.result == CaptureResult::Finished {
            return Ok(());
        }
        let capture = captured
            .capture
            .ok_or_else(|| JobError::retryable(anyhow::anyhow!("playlist capture is missing")))?;
        let token = match self
            .connections
            .access_token(&self.token_client, &capture.owner_id)
            .await
        {
            Ok(token) => token,
            Err(error) => return self.finish_connection_failure(&capture, error).await,
        };
        if let Some(app_id) = token.oauth_app_id
            && let Some(retry_after_seconds) = self
                .connections
                .app_retry_after_seconds(app_id)
                .await
                .map_err(connection_job_error)?
        {
            let retry = seconds(retry_after_seconds, RATE_LIMIT_RETRY);
            return self
                .finish_failure(&capture, cooling_down_failure(retry))
                .await;
        }
        let metadata_observation = catalog_ingest::Observation::begin(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        let snapshot = match self.reader.observe(&urn, &token.value).await {
            Ok(snapshot) => snapshot,
            Err(error) if is_unauthorized(&error) => {
                let refreshed = match self
                    .connections
                    .refresh_rejected_token(&self.token_client, &capture.owner_id, &token.value)
                    .await
                {
                    Ok(token) => token,
                    Err(error) => return self.finish_connection_failure(&capture, error).await,
                };
                match self.reader.observe(&urn, &refreshed.value).await {
                    Ok(snapshot) => snapshot,
                    Err(error) if is_unauthorized(&error) => {
                        self.connections
                            .reject_for_later(&capture.owner_id, &refreshed.value)
                            .await
                            .map_err(connection_job_error)?;
                        return self.finish_failure(&capture, unauthorized_failure()).await;
                    }
                    Err(error) => {
                        return self
                            .finish_read_failure(&capture, error, refreshed.oauth_app_id)
                            .await;
                    }
                }
            }
            Err(error) => {
                return self
                    .finish_read_failure(&capture, error, token.oauth_app_id)
                    .await;
            }
        };
        let result = self
            .repository
            .persist_success(&capture, &snapshot, metadata_observation)
            .await
            .map_err(repository_job_error)?;
        if result == PersistResult::Applied {
            for track in &snapshot.hydrated_tracks {
                if let Err(error) = wake::enqueue(&self.pool, &self.queue, &track.sc_track_id).await
                {
                    tracing::debug!(track = %track.sc_track_id, %error, "lyrics wake deferred to sweep");
                }
            }
        }
        Ok(())
    }

    async fn finish_connection_failure(
        &self,
        capture: &ObservationCapture,
        error: ConnectionError,
    ) -> JobResult {
        match error {
            ConnectionError::Database(error) => Err(JobError::retryable(anyhow::anyhow!(
                "playlist connection lookup failed: {error}"
            ))),
            ConnectionError::ReauthorizationRequired => {
                self.finish_failure(capture, unauthorized_failure()).await
            }
            ConnectionError::RateLimited {
                retry_after_seconds,
            } => {
                self.finish_failure(
                    capture,
                    rate_limited_failure(seconds(retry_after_seconds, RATE_LIMIT_RETRY)),
                )
                .await
            }
            ConnectionError::RefreshInProgress {
                retry_after_seconds,
            }
            | ConnectionError::TemporarilyUnavailable {
                retry_after_seconds,
            } => {
                self.finish_failure(
                    capture,
                    transport_failure(
                        "soundcloud_connection_temporarily_unavailable",
                        seconds(retry_after_seconds, TRANSPORT_RETRY),
                    ),
                )
                .await
            }
        }
    }

    async fn finish_read_failure(
        &self,
        capture: &ObservationCapture,
        error: PlaylistObserveError,
        oauth_app_id: Option<Uuid>,
    ) -> JobResult {
        let failure = read_failure(&error);
        if let Some(oauth_app_id) = oauth_app_id
            && let Some(minimum_seconds) = app_wide_cooldown(&failure)
            && let Err(error) = self
                .connections
                .penalize_app(oauth_app_id, minimum_seconds)
                .await
        {
            tracing::warn!(%error, "playlist observation could not publish the oauth app cooldown");
        }
        self.finish_failure(capture, failure).await
    }

    async fn finish_failure(
        &self,
        capture: &ObservationCapture,
        failure: FailureObservation,
    ) -> JobResult {
        self.repository
            .persist_failure(capture, &failure)
            .await
            .map_err(repository_job_error)?;
        Ok(())
    }
}

fn app_wide_cooldown(failure: &FailureObservation) -> Option<i64> {
    let minimum = match failure.error_kind.as_str() {
        "soundcloud_rate_limited" => RATE_LIMIT_RETRY,
        "soundcloud_forbidden" => FORBIDDEN_RETRY,
        "soundcloud_playlist_server_error" => TRANSPORT_RETRY,
        _ => return None,
    };
    i64::try_from(minimum.as_secs()).ok()
}

fn read_failure(error: &PlaylistObserveError) -> FailureObservation {
    match error {
        PlaylistObserveError::Read(PlaylistReadError::Api { status, .. })
            if *status == StatusCode::NOT_FOUND =>
        {
            not_found_failure()
        }
        PlaylistObserveError::Read(PlaylistReadError::Api {
            status,
            retry_after_seconds,
            ..
        }) if *status == StatusCode::TOO_MANY_REQUESTS => rate_limited_failure(seconds(
            retry_after_seconds.unwrap_or(RATE_LIMIT_RETRY.as_secs() as i64),
            RATE_LIMIT_RETRY,
        )),
        PlaylistObserveError::Read(PlaylistReadError::Api { status, .. })
            if *status == StatusCode::UNAUTHORIZED =>
        {
            unauthorized_failure()
        }
        PlaylistObserveError::Read(PlaylistReadError::Api { status, .. })
            if *status == StatusCode::FORBIDDEN =>
        {
            transport_failure("soundcloud_forbidden", FORBIDDEN_RETRY)
        }
        PlaylistObserveError::Read(
            PlaylistReadError::InvalidTarget
            | PlaylistReadError::InvalidJson(_)
            | PlaylistReadError::ResponseTooLarge,
        )
        | PlaylistObserveError::Invalid(_) => malformed_failure("soundcloud_playlist_malformed"),
        PlaylistObserveError::Deadline => {
            transport_failure("soundcloud_playlist_deadline", TRANSPORT_RETRY)
        }
        PlaylistObserveError::Read(PlaylistReadError::Api { status, .. })
            if status.is_server_error() =>
        {
            transport_failure("soundcloud_playlist_server_error", TRANSPORT_RETRY)
        }
        PlaylistObserveError::Read(PlaylistReadError::Transport(_))
        | PlaylistObserveError::Read(PlaylistReadError::Api { .. }) => {
            transport_failure("soundcloud_playlist_unavailable", TRANSPORT_RETRY)
        }
    }
}

fn unauthorized_failure() -> FailureObservation {
    failure(
        "unauthorized",
        "auth_required",
        "auth_required",
        None,
        Some(REAUTHORIZATION_RETRY),
        "soundcloud_reauthorization_required",
    )
}

fn cooling_down_failure(retry: Duration) -> FailureObservation {
    failure(
        "rate_limited",
        "retry_wait",
        "retry_wait",
        None,
        Some(retry),
        "soundcloud_app_cooling_down",
    )
}

fn rate_limited_failure(retry: Duration) -> FailureObservation {
    failure(
        "rate_limited",
        "retry_wait",
        "retry_wait",
        None,
        Some(retry),
        "soundcloud_rate_limited",
    )
}

fn transport_failure(error_kind: &'static str, retry: Duration) -> FailureObservation {
    failure(
        "transport",
        "retry_wait",
        "retry_wait",
        None,
        Some(retry),
        error_kind,
    )
}

fn malformed_failure(error_kind: &'static str) -> FailureObservation {
    failure(
        "malformed",
        "incomplete",
        "retry_wait",
        None,
        Some(MALFORMED_RETRY),
        error_kind,
    )
}

fn not_found_failure() -> FailureObservation {
    failure(
        "not_found",
        "conflict",
        "conflict",
        Some("remote_not_found"),
        Some(FORBIDDEN_RETRY),
        "soundcloud_playlist_not_found",
    )
}

fn failure(
    outcome: &'static str,
    run_decision: &'static str,
    state_status: &'static str,
    conflict_code: Option<&'static str>,
    retry: Option<Duration>,
    error_kind: &'static str,
) -> FailureObservation {
    let observed_at = Utc::now();
    FailureObservation {
        outcome,
        run_decision,
        state_status,
        conflict_code,
        retry_at: retry.map(|retry| {
            let seconds = i64::try_from(retry.as_secs()).unwrap_or(i64::MAX);
            observed_at + chrono::Duration::seconds(seconds)
        }),
        error_kind: error_kind.to_owned(),
        observed_at,
    }
}

fn seconds(value: i64, fallback: Duration) -> Duration {
    u64::try_from(value)
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(fallback)
        .min(MAX_REMOTE_RETRY)
}

fn is_unauthorized(error: &PlaylistObserveError) -> bool {
    error.read().is_some_and(PlaylistReadError::is_unauthorized)
}

fn repository_job_error(error: RepositoryError) -> JobError {
    match error {
        RepositoryError::MissingState | RepositoryError::MissingOwner => JobError::permanent(error),
        RepositoryError::InconsistentSnapshot
        | RepositoryError::InvalidHydration(_)
        | RepositoryError::Database(_) => JobError::retryable(error),
    }
}

fn connection_job_error(error: ConnectionError) -> JobError {
    JobError::retryable(anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_finishes_without_erasing_local_membership() {
        let failure = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::NOT_FOUND,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        }));

        assert_eq!(failure.state_status, "conflict");
        assert_eq!(failure.conflict_code, Some("remote_not_found"));
    }

    #[test]
    fn rate_limit_preserves_remote_retry_hint() {
        let failure = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: serde_json::Value::Null,
            retry_after_seconds: Some(17),
        }));

        assert_eq!(
            failure.retry_at,
            Some(failure.observed_at + chrono::Duration::seconds(17))
        );
    }

    #[test]
    fn retry_hint_is_bounded_by_policy() {
        let failure = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: serde_json::Value::Null,
            retry_after_seconds: Some(i64::MAX),
        }));

        assert_eq!(
            failure.retry_at,
            Some(failure.observed_at + chrono::Duration::hours(24))
        );
    }

    #[test]
    fn every_durable_failure_leaves_the_retry_to_its_own_due_time() {
        let failures = [
            read_failure(&PlaylistObserveError::Deadline),
            read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
                status: StatusCode::UNAUTHORIZED,
                body: serde_json::Value::Null,
                retry_after_seconds: None,
            })),
            read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
                status: StatusCode::FORBIDDEN,
                body: serde_json::Value::Null,
                retry_after_seconds: None,
            })),
        ];

        for failure in failures {
            assert!(failure.retry_at.is_some());
        }
    }

    #[test]
    fn only_app_wide_soundcloud_failures_penalize_the_shared_cooldown() {
        let rate_limited = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        }));
        let forbidden = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::FORBIDDEN,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        }));
        let server_error = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::BAD_GATEWAY,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        }));
        let unreachable = read_failure(&PlaylistObserveError::Deadline);
        let not_found = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::NOT_FOUND,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        }));
        let unauthorized = read_failure(&PlaylistObserveError::Read(PlaylistReadError::Api {
            status: StatusCode::UNAUTHORIZED,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        }));

        assert_eq!(app_wide_cooldown(&rate_limited), Some(300));
        assert_eq!(app_wide_cooldown(&forbidden), Some(1800));
        assert_eq!(app_wide_cooldown(&server_error), Some(120));
        assert_eq!(app_wide_cooldown(&unreachable), None);
        assert_eq!(app_wide_cooldown(&not_found), None);
        assert_eq!(app_wide_cooldown(&unauthorized), None);
    }
}

#[cfg(test)]
mod integration_tests;
