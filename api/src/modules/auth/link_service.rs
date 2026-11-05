use std::sync::Arc;

use chrono::NaiveDateTime;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::AuthService;
use crate::modules::auth::start_limit::starts_limited;

pub struct LinkService {
    pool: PgPool,
    auth: Arc<AuthService>,
}

pub struct CreateLinkResult {
    pub link_request_id: Uuid,
    pub claim_token: String,
    pub expires_at: NaiveDateTime,
}

pub struct ClaimResult {
    pub session_id: Uuid,
    pub mode: String,
}

pub struct LinkStatusResult {
    pub status: String,
    pub mode: String,
    pub session_id: Option<Uuid>,
    pub error: Option<String>,
}

impl LinkService {
    pub fn new(pool: PgPool, auth: Arc<AuthService>) -> Arc<Self> {
        Arc::new(Self { pool, auth })
    }

    pub async fn create(
        &self,
        mode: &str,
        source_session_id: Option<Uuid>,
    ) -> AppResult<CreateLinkResult> {
        self.create_bound(mode, source_session_id, None, |_| false)
            .await
    }

    pub async fn create_bound(
        &self,
        mode: &str,
        source_session_id: Option<Uuid>,
        caller_bind: Option<&[u8]>,
        owns_bind: impl Fn(&[u8]) -> bool,
    ) -> AppResult<CreateLinkResult> {
        validate_mode(mode, source_session_id)?;
        if let Some(session_id) = source_session_id {
            let session = self
                .auth
                .find_session(session_id)
                .await?
                .ok_or_else(|| AppError::unauthorized("Source session not found"))?;
            if session.soundcloud_connection_id.is_none() {
                return Err(AppError::soundcloud_reauthorization_required());
            }
        }
        let chat_bind = match (mode, caller_bind) {
            (_, None) => None,
            ("pull", Some(bind)) => Some(bind.to_vec()),
            (_, Some(_)) => self.source_proof(source_session_id, owns_bind).await?,
        };

        let created = sqlx::query_file!(
            "queries/auth/link_service/insert.sql",
            mode,
            source_session_id,
            chat_bind
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(starts_limited)?
        .ok_or_else(|| AppError::unauthorized("Source session not found"))?;

        Ok(CreateLinkResult {
            link_request_id: created.link_request_id,
            claim_token: created.claim_token,
            expires_at: created.expires_at,
        })
    }

    pub async fn claim(
        &self,
        claim_token: &str,
        caller_session_id: Option<Uuid>,
    ) -> AppResult<ClaimResult> {
        self.claim_bound(claim_token, caller_session_id, None, |_| false)
            .await
    }

    pub async fn claim_bound(
        &self,
        claim_token: &str,
        caller_session_id: Option<Uuid>,
        caller_bind: Option<&[u8]>,
        owns_bind: impl Fn(&[u8]) -> bool,
    ) -> AppResult<ClaimResult> {
        let proven_bind = match caller_bind {
            Some(_) => self.source_proof(caller_session_id, owns_bind).await?,
            None => None,
        };
        let claimed = sqlx::query_file!(
            "queries/auth/link_service/claim.sql",
            claim_token,
            caller_session_id,
            caller_bind,
            proven_bind.as_deref()
        )
        .fetch_one(&self.pool)
        .await?;
        match (claimed.outcome.as_str(), claimed.session_id, claimed.mode) {
            ("claimed", Some(session_id), Some(mode)) => {
                info!("Link claimed");
                Ok(ClaimResult { session_id, mode })
            }
            (outcome, _, _) => Err(claim_error(outcome)),
        }
    }

    pub async fn get_status(&self, link_request_id: Uuid) -> AppResult<LinkStatusResult> {
        self.get_status_for(link_request_id, |_| false).await
    }

    pub async fn get_status_for(
        &self,
        link_request_id: Uuid,
        owns_bind: impl Fn(&[u8]) -> bool,
    ) -> AppResult<LinkStatusResult> {
        let link = sqlx::query_file!("queries/auth/link_service/status.sql", link_request_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(link) = link else {
            return Ok(LinkStatusResult {
                status: "expired".to_owned(),
                mode: "pull".to_owned(),
                session_id: None,
                error: Some("Unknown link request".to_owned()),
            });
        };
        let session_id = link
            .target_session_id
            .filter(|_| link.chat_bind.as_deref().is_none_or(&owns_bind));
        Ok(LinkStatusResult {
            status: link.status,
            mode: link.mode,
            session_id,
            error: link.error,
        })
    }

    async fn source_proof(
        &self,
        source_session_id: Option<Uuid>,
        owns_bind: impl Fn(&[u8]) -> bool,
    ) -> AppResult<Option<Vec<u8>>> {
        let Some(session_id) = source_session_id else {
            return Ok(None);
        };
        let stored =
            sqlx::query_file_scalar!("queries/auth/link_service/session_bind.sql", session_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten();
        Ok(stored.filter(|bind| owns_bind(bind)))
    }
}

fn claim_error(outcome: &str) -> AppError {
    match outcome {
        "unknown" => AppError::not_found("Invalid or already used link token"),
        "expired" => AppError::bad_request("Link token expired"),
        "pull_needs_source" => AppError::unauthorized("pull claim requires source session"),
        "push_without_source" => AppError::bad_request("push link has no source session"),
        "source_missing" => AppError::unauthorized("Source session not found"),
        "reauthorize" => AppError::soundcloud_reauthorization_required(),
        _ => AppError::bad_request("Unknown link mode"),
    }
}

fn validate_mode(mode: &str, source_session_id: Option<Uuid>) -> AppResult<()> {
    match (mode, source_session_id) {
        ("push", Some(_)) | ("pull", None) => Ok(()),
        ("push", None) => Err(AppError::bad_request("push mode requires source session")),
        ("pull", Some(_)) => Err(AppError::bad_request(
            "pull mode must not have source session at creation",
        )),
        _ => Err(AppError::bad_request("mode must be 'pull' or 'push'")),
    }
}
