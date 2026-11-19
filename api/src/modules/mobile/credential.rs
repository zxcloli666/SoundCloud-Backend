use axum::http::StatusCode;
use backend_contracts::auth::{RefreshOutcome, RefreshRequest, RefreshTarget};
use chrono::{TimeDelta, Utc};
use tracing::warn;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

use super::dto::MobileOverview;
use super::exchange::Linked;
use super::service::MobileService;

const ACTIVE: &str = "active";
const RELINK_REQUIRED: &str = "relink_required";
const REFRESH_DUE_BUFFER: TimeDelta = TimeDelta::minutes(5);

impl MobileService {
    pub(super) async fn store(
        &self,
        request_id: Uuid,
        oauth_app_id: Uuid,
        device_id: Uuid,
        linked: &Linked,
    ) -> AppResult<()> {
        let account = linked.soundcloud_user_id.as_str();
        let mut transaction = self.pool.begin().await?;
        sqlx::query_file_scalar!("queries/mobile/lock_account.sql", account)
            .fetch_one(&mut *transaction)
            .await?;
        let link_v = sqlx::query_file_scalar!("queries/mobile/lock_credential.sql", account)
            .fetch_optional(&mut *transaction)
            .await?;
        match link_v {
            Some(link_v) => {
                wt::call(
                    &mut transaction,
                    &wt::MobileCredentialLink {
                        soundcloud_user_id: account,
                        oauth_app_id,
                        device_id,
                        access_token: linked.access_token.expose().as_str(),
                        refresh_token: linked.refresh_token.expose().as_str(),
                        expires_at: linked.expires_at,
                        scope: &linked.scope,
                        link_v,
                        at: wt::now(),
                    },
                )
                .await?;
            }
            None => {
                sqlx::query_file!(
                    "queries/mobile/insert_credential.sql",
                    account,
                    oauth_app_id,
                    device_id,
                    linked.access_token.expose(),
                    linked.refresh_token.expose(),
                    linked.expires_at,
                    &linked.scope
                )
                .execute(&mut *transaction)
                .await?;
            }
        }
        if let Some(plan) = &linked.plan {
            sqlx::query_file!("queries/mobile/set_plan.sql", account, plan.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        let completed = sqlx::query_file!(
            "queries/mobile/finish_request.sql",
            request_id,
            "completed",
            None::<&str>
        )
        .execute(&mut *transaction)
        .await?;
        if completed.rows_affected() == 0 {
            return Err(AppError::internal(
                "mobile link request changed while linking",
            ));
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn unlink(&self, account: &str) -> AppResult<()> {
        let mut transaction = self.pool.begin().await?;
        wt::call(
            &mut transaction,
            &wt::MobileCredentialDrop {
                soundcloud_user_id: account,
            },
        )
        .await?;
        transaction.commit().await?;
        crate::metrics::record_mobile_link("unlinked");
        Ok(())
    }

    pub async fn overview(&self, session_id: Uuid, account: &str) -> AppResult<MobileOverview> {
        let available = self.allow.admits(account);
        let credential = sqlx::query_file!("queries/mobile/credential_of.sql", account)
            .fetch_optional(&self.pool)
            .await?;
        let Some(credential) = credential else {
            return Ok(MobileOverview {
                available,
                linked: false,
                status: None,
                plan: None,
                plan_seen_at: None,
                uq_opt_in: false,
            });
        };
        let due = credential.expires_at <= Utc::now() + REFRESH_DUE_BUFFER;
        let status = if credential.status == ACTIVE && due {
            self.refreshed_status(session_id)
                .await
                .unwrap_or(credential.status)
        } else {
            credential.status
        };
        Ok(MobileOverview {
            available,
            linked: true,
            status: Some(status),
            plan: credential.plan,
            plan_seen_at: credential.plan_seen_at,
            uq_opt_in: credential.uq_opt_in,
        })
    }

    pub async fn choose_uq(
        &self,
        session_id: Uuid,
        account: &str,
        opt_in: bool,
    ) -> AppResult<MobileOverview> {
        let chosen = sqlx::query_file_scalar!("queries/mobile/set_uq.sql", account, opt_in)
            .fetch_optional(&self.pool)
            .await?;
        if chosen.is_none() {
            return Err(AppError::coded(
                StatusCode::NOT_FOUND,
                "mobile_not_linked",
                "No mobile profile is linked to this account",
            ));
        }
        self.overview(session_id, account).await
    }

    async fn refreshed_status(&self, session_id: Uuid) -> Option<String> {
        let session = self.auth.find_session_cached(session_id).await.ok()??;
        let connection_id = session.soundcloud_connection_id?;
        let request = RefreshRequest {
            connection_id,
            rejected_token_fingerprint: None,
            hold: false,
            target: RefreshTarget::Mobile,
        };
        let reply = match self.core.refresh(&request).await {
            Ok(reply) => reply,
            Err(error) => {
                warn!(%error, "mobile refresh did not reach core");
                return None;
            }
        };
        if reply.outcome == RefreshOutcome::ReauthorizationRequired {
            return Some(RELINK_REQUIRED.to_owned());
        }
        reply.mobile.map(|state| state.status)
    }
}
