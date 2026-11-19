use backend_contracts::auth::{RefreshOutcome, RefreshRequest, RefreshTarget, token_fingerprint};
use chrono::{TimeDelta, Utc};
use redact::Secret;
use sc_mobile::{Caller, MobileClient, MobileError, MobileReply, MobileRequest};
use tracing::warn;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

use super::credential::RELINK_REQUIRED;
use super::refusal::{captcha, not_linked, relink_required, unavailable};
use super::service::MobileService;

const UNAUTHORIZED: u16 = 401;
const REFRESH_DUE_BUFFER: TimeDelta = TimeDelta::minutes(5);

struct Standing {
    device_id: String,
    token: Secret<String>,
    expires_at: chrono::DateTime<Utc>,
}

impl MobileService {
    pub(super) async fn owner_reply(
        &self,
        session_id: Uuid,
        account: &str,
        op: &'static str,
        request: &MobileRequest,
    ) -> AppResult<MobileReply> {
        let row = sqlx::query_file!("queries/mobile/session_of.sql", account)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(not_linked)?;
        if row.status == RELINK_REQUIRED {
            return Err(relink_required());
        }
        let profile = self
            .profile(row.oauth_app_id)
            .await?
            .ok_or_else(unavailable)?;
        let client = self.client(profile.app)?;
        let connection_id = self.connection_of(session_id).await?;
        let mut standing = Standing {
            device_id: row.device_id.simple().to_string(),
            token: Secret::new(row.access_token),
            expires_at: row.expires_at,
        };
        if standing.expires_at <= Utc::now() + REFRESH_DUE_BUFFER
            && let Some(token) = self.renewed(connection_id, None).await?
        {
            standing.token = token;
        }
        let reply = send(&client, &standing, op, request).await?;
        if reply.status != UNAUTHORIZED {
            return judged(reply);
        }
        let rejected = token_fingerprint(standing.token.expose());
        standing.token = self
            .renewed(connection_id, Some(rejected))
            .await?
            .ok_or_else(unavailable)?;
        let reply = send(&client, &standing, op, request).await?;
        if reply.status == UNAUTHORIZED {
            return Err(self.require_relink(account, standing.token.expose()).await);
        }
        judged(reply)
    }

    pub(super) async fn require_relink(&self, account: &str, rejected_token: &str) -> AppError {
        let marked = sqlx::query_file!(
            "queries/mobile/relink_required.sql",
            account,
            rejected_token
        )
        .execute(&self.pool)
        .await;
        if let Err(error) = marked {
            warn!(%error, "mobile credential was not marked for relink");
        }
        relink_required()
    }

    async fn connection_of(&self, session_id: Uuid) -> AppResult<Uuid> {
        let session = self
            .auth
            .find_session_cached(session_id)
            .await?
            .ok_or_else(not_linked)?;
        session.soundcloud_connection_id.ok_or_else(not_linked)
    }

    async fn renewed(
        &self,
        connection_id: Uuid,
        rejected: Option<String>,
    ) -> AppResult<Option<Secret<String>>> {
        let request = RefreshRequest {
            connection_id,
            rejected_token_fingerprint: rejected,
            hold: false,
            target: RefreshTarget::Mobile,
        };
        let reply = match self.core.refresh(&request).await {
            Ok(reply) => reply,
            Err(error) => {
                warn!(%error, "mobile refresh did not reach core");
                return Ok(None);
            }
        };
        match reply.outcome {
            RefreshOutcome::ReauthorizationRequired => Err(relink_required()),
            RefreshOutcome::NotConnected => Err(not_linked()),
            RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh => Ok(reply
                .mobile
                .and_then(|state| state.access_token)
                .map(Secret::new)),
            _ => Ok(None),
        }
    }
}

async fn send(
    client: &MobileClient,
    standing: &Standing,
    op: &'static str,
    request: &MobileRequest,
) -> AppResult<MobileReply> {
    let caller = Caller::user(&standing.device_id, standing.token.clone());
    let answer = client.call(&caller, request).await;
    let class = match &answer {
        Ok(reply) => reply.status_class(),
        Err(error) => error.status_class(),
    };
    crate::metrics::record_mobile_request(op, class);
    answer.map_err(|error| {
        warn!(%error, "mobile request failed");
        match error {
            MobileError::Captcha => captcha(),
            _ => unavailable(),
        }
    })
}

fn judged(reply: MobileReply) -> AppResult<MobileReply> {
    if reply.is_captcha() {
        return Err(captcha());
    }
    if !reply.is_success() {
        return Err(unavailable());
    }
    Ok(reply)
}
