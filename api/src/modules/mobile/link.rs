use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use redact::Secret;
use sc_mobile::{MobileError, Pkce, authorize_url, new_state};
use tokio::time::Instant;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::service::replica::Feed;

use super::dto::{LinkAccount, LinkStarted, LinkStatus};
use super::exchange::{Exchanged, exchange};
use super::service::MobileService;

const REQUEST_TTL_SECONDS: i32 = 15 * 60;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(25);
const REQUEST_WAIT: Duration = Duration::from_secs(10);
const REQUEST_POLL: Duration = Duration::from_millis(250);
const IN_FLIGHT: Duration = Duration::from_secs(60);
const REPLICA_RETRY_SECONDS: i64 = 2;
const PENDING: &str = "pending";
const EXPIRED: &str = "expired";

struct Claimed {
    id: Uuid,
    code_verifier: String,
    oauth_app_id: Uuid,
    expected_soundcloud_user_id: String,
    device_id: Uuid,
}

struct Seen {
    state: String,
    status: String,
    error: Option<String>,
    expired: bool,
}

enum Finished {
    Completed,
    Mismatch,
    Failed(&'static str),
}

impl MobileService {
    pub async fn start(&self, session_id: Uuid, account: &str) -> AppResult<LinkStarted> {
        if !self.allow.admits(account) {
            return Err(AppError::coded(
                StatusCode::FORBIDDEN,
                "mobile_link_unavailable",
                "Linking a mobile profile is not available for this account",
            ));
        }
        let Some(profile) = self.picked_profile().await? else {
            return Err(AppError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "mobile_profile_unavailable",
                "No mobile profile is configured",
            ));
        };
        let owner = sqlx::query_file!("queries/mobile/account.sql", session_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session has no SoundCloud account"))?;
        let device_id = sqlx::query_file_scalar!("queries/mobile/device_of.sql", account)
            .fetch_optional(&self.pool)
            .await?
            .unwrap_or_else(Uuid::new_v4);
        let pkce = Pkce::generate().map_err(refused_locally)?;
        let state = new_state().map_err(refused_locally)?;
        let url = authorize_url(
            &profile.app,
            pkce.challenge(),
            state.expose(),
            &device_id.simple().to_string(),
        )
        .map_err(refused_locally)?;
        let id = Uuid::now_v7();
        sqlx::query_file!(
            "queries/mobile/create_request.sql",
            id,
            state.expose(),
            pkce.verifier().expose(),
            profile.id,
            session_id,
            account,
            device_id,
            REQUEST_TTL_SECONDS
        )
        .execute(&self.pool)
        .await?;
        crate::metrics::record_mobile_link("started");
        Ok(LinkStarted {
            id,
            url: url.to_string(),
            account: LinkAccount {
                username: owner.username,
                avatar_url: owner.avatar_url,
            },
        })
    }

    pub async fn callback(
        &self,
        session_id: Uuid,
        account: &str,
        id: Uuid,
        code: &Secret<String>,
        state: &Secret<String>,
    ) -> AppResult<LinkStatus> {
        let feed = self.auth.feed().await?;
        let deadline = Instant::now() + REQUEST_WAIT;
        loop {
            if let Some(claimed) = self.claim(id, session_id, state).await? {
                return self.finish(account, claimed, code).await;
            }
            if let Some(seen) = self.seen(id, session_id).await? {
                return seen.answer(state);
            }
            if feed == Feed::Local || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(REQUEST_POLL).await;
        }
        Err(self.missing(id).await?)
    }

    pub async fn status(&self, session_id: Uuid, id: Uuid) -> AppResult<LinkStatus> {
        match self.seen(id, session_id).await? {
            Some(seen) => Ok(seen.shown()),
            None => Err(self.missing(id).await?),
        }
    }

    async fn claim(
        &self,
        id: Uuid,
        session_id: Uuid,
        state: &Secret<String>,
    ) -> AppResult<Option<Claimed>> {
        Ok(sqlx::query_file_as!(
            Claimed,
            "queries/mobile/claim_request.sql",
            id,
            session_id,
            state.expose()
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn seen(&self, id: Uuid, session_id: Uuid) -> AppResult<Option<Seen>> {
        Ok(
            sqlx::query_file_as!(Seen, "queries/mobile/request_of.sql", id, session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn missing(&self, id: Uuid) -> AppResult<AppError> {
        let feed = self.auth.feed().await?;
        let may_arrive = feed == Feed::Stalled || (feed == Feed::Streaming && minted_recently(id));
        if may_arrive {
            return Ok(AppError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "mobile_link_not_replicated",
                "This server has not received the link request yet, retry shortly",
            )
            .with_retry_after(REPLICA_RETRY_SECONDS));
        }
        Ok(AppError::coded(
            StatusCode::NOT_FOUND,
            "mobile_link_unknown",
            "Unknown mobile link request",
        ))
    }

    async fn finish(
        &self,
        account: &str,
        claimed: Claimed,
        code: &Secret<String>,
    ) -> AppResult<LinkStatus> {
        let id = claimed.id;
        let finished =
            match tokio::time::timeout(CALLBACK_TIMEOUT, self.link(account, claimed, code)).await {
                Ok(Ok(finished)) => finished,
                Ok(Err(failure)) => {
                    error!(error = %failure, "mobile link failed");
                    Finished::Failed("internal")
                }
                Err(_) => {
                    warn!("mobile link timed out");
                    Finished::Failed("timed_out")
                }
            };
        crate::metrics::record_mobile_link(finished.outcome());
        let (status, error) = finished.recorded();
        if !matches!(finished, Finished::Completed) {
            sqlx::query_file!("queries/mobile/finish_request.sql", id, status, error)
                .execute(&self.pool)
                .await?;
        }
        Ok(LinkStatus {
            status: status.to_owned(),
            error: error.map(str::to_owned),
        })
    }

    async fn link(
        &self,
        account: &str,
        claimed: Claimed,
        code: &Secret<String>,
    ) -> AppResult<Finished> {
        if claimed.expected_soundcloud_user_id != account {
            return Ok(Finished::Failed("session_changed"));
        }
        let Some(profile) = self.profile(claimed.oauth_app_id).await? else {
            return Ok(Finished::Failed("profile_unavailable"));
        };
        let client = self.client(profile.app)?;
        let device = claimed.device_id.simple().to_string();
        let verifier = Secret::new(claimed.code_verifier);
        let linked = match exchange(&client, code, &verifier, &device).await {
            Exchanged::Account(linked) => linked,
            Exchanged::Failed(reason) => return Ok(Finished::Failed(reason)),
        };
        if linked.soundcloud_user_id != claimed.expected_soundcloud_user_id {
            info!("mobile link refused: the signed-in profile is another account");
            return Ok(Finished::Mismatch);
        }
        self.store(claimed.id, profile.id, claimed.device_id, &linked)
            .await?;
        Ok(Finished::Completed)
    }
}

impl Seen {
    fn answer(self, state: &Secret<String>) -> AppResult<LinkStatus> {
        if self.state != *state.expose() {
            return Err(AppError::coded(
                StatusCode::BAD_REQUEST,
                "mobile_link_state",
                "The link state does not match this request",
            ));
        }
        if self.status == PENDING && self.expired {
            return Err(AppError::coded(
                StatusCode::GONE,
                "mobile_link_expired",
                "This link request has expired, start a new one",
            ));
        }
        Ok(self.shown())
    }

    fn shown(self) -> LinkStatus {
        let status = if self.status == PENDING && self.expired {
            EXPIRED.to_owned()
        } else {
            self.status
        };
        LinkStatus {
            status,
            error: self.error,
        }
    }
}

impl Finished {
    fn outcome(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Mismatch => "account_mismatch",
            Self::Failed("timed_out") => "timed_out",
            Self::Failed(_) => "failed",
        }
    }

    fn recorded(&self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::Completed => ("completed", None),
            Self::Mismatch => ("account_mismatch", None),
            Self::Failed(reason) => ("failed", Some(*reason)),
        }
    }
}

fn refused_locally(error: MobileError) -> AppError {
    AppError::internal(format!("mobile link could not be prepared: {error}"))
}

fn minted_recently(id: Uuid) -> bool {
    let Some((seconds, nanos)) = id.get_timestamp().map(|minted| minted.to_unix()) else {
        return false;
    };
    UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanos))
        .and_then(|minted| SystemTime::now().duration_since(minted).ok())
        .is_some_and(|age| age < IN_FLIGHT)
}
