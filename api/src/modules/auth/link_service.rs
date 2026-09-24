use std::sync::Arc;

use base64::Engine;
use chrono::{NaiveDateTime, Utc};
use rand::RngCore;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::AuthService;
use crate::modules::auth::model::LinkRequestRow;

const LINK_REQUEST_TTL_SECS: i64 = 5 * 60;

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
        validate_mode(mode, source_session_id)?;
        if let Some(session_id) = source_session_id {
            let session = self
                .auth
                .get_session(session_id)
                .await?
                .ok_or_else(|| AppError::unauthorized("Source session not found"))?;
            if session.soundcloud_connection_id.is_none() {
                return Err(AppError::soundcloud_reauthorization_required());
            }
        }

        let mut bytes = [0; 24];
        rand::thread_rng().fill_bytes(&mut bytes);
        let claim_token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(LINK_REQUEST_TTL_SECS)).naive_utc();
        let row = sqlx::query_file_as!(
            LinkRequestRow,
            "queries/auth/link_service/insert.sql",
            Uuid::new_v4(),
            claim_token,
            mode,
            source_session_id,
            expires_at
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(CreateLinkResult {
            link_request_id: row.id,
            claim_token,
            expires_at: row.expires_at,
        })
    }

    pub async fn claim(
        &self,
        claim_token: &str,
        caller_session_id: Option<Uuid>,
    ) -> AppResult<ClaimResult> {
        let mut transaction = self.pool.begin().await?;
        let link = sqlx::query_file_as!(
            LinkRequestRow,
            "queries/auth/link_service/lock_pending_by_claim_token.sql",
            claim_token
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::not_found("Invalid or already used link token"))?;

        if link.expires_at < Utc::now().naive_utc() {
            sqlx::query_file!(
                "queries/auth/link_service/mark_expired.sql",
                link.id,
                "Expired"
            )
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
            return Err(AppError::bad_request("Link token expired"));
        }

        let source_session_id = match link.mode.as_str() {
            "pull" => caller_session_id
                .ok_or_else(|| AppError::unauthorized("pull claim requires source session"))?,
            "push" => link
                .source_session_id
                .ok_or_else(|| AppError::bad_request("push link has no source session"))?,
            _ => return Err(AppError::bad_request("Unknown link mode")),
        };
        let source = sqlx::query_file!(
            "queries/auth/service/lock_session_connection.sql",
            source_session_id
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::unauthorized("Source session not found"))?;
        let connection_id = source
            .soundcloud_connection_id
            .ok_or_else(AppError::soundcloud_reauthorization_required)?;

        let target_session_id = Uuid::new_v4();
        sqlx::query_file!(
            "queries/auth/link_service/insert_target_session.sql",
            target_session_id,
            connection_id
        )
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query_file!(
            "queries/auth/link_service/mark_claimed.sql",
            link.id,
            source_session_id,
            target_session_id
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        info!("Link claimed");
        Ok(ClaimResult {
            session_id: target_session_id,
            mode: link.mode,
        })
    }

    pub async fn get_status(&self, link_request_id: Uuid) -> AppResult<LinkStatusResult> {
        let link = sqlx::query_file_as!(
            LinkRequestRow,
            "queries/auth/link_service/by_id.sql",
            link_request_id
        )
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
        if link.status == "pending" && link.expires_at < Utc::now().naive_utc() {
            return Ok(LinkStatusResult {
                status: "expired".to_owned(),
                mode: link.mode,
                session_id: None,
                error: Some("Expired".to_owned()),
            });
        }

        let session_id = (link.mode == "pull" && link.status == "claimed")
            .then_some(link.target_session_id)
            .flatten();
        Ok(LinkStatusResult {
            status: link.status,
            mode: link.mode,
            session_id,
            error: link.error,
        })
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
