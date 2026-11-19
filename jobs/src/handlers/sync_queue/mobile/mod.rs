mod created;
mod playlist;
mod request;
mod session;
#[cfg(test)]
mod shape_tests;
#[cfg(test)]
mod tests;
mod verdict;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use backend_contracts::auth::{RefreshOutcome, token_fingerprint};
use redact::Secret;
use sc_mobile::{Caller, Egress, MobileApp, MobileClient, MobileReply, MobileRequest};
use sqlx::PgPool;
use tracing::warn;
use url::Url;
use uuid::Uuid;

use super::actions::ActionError;
use super::client::SoundCloudError;
use super::write_gate::{WriteGate, WriteHold};
use crate::handlers::mobile_refresh::MobileRefresh;
use session::{RELINK_REQUIRED, Session};

pub(super) use created::shape;
pub(super) use playlist::carries_only_sent_fields;
pub(super) use request::build;
pub(super) use verdict::judge;

const UNAUTHORIZED: u16 = 401;
const REFRESH_DUE_SECONDS: i64 = 5 * 60;
const RENEWAL_RETRY_SECONDS: i64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Apiv1,
    Mobile,
    MobileThenApiv1,
}

pub(super) fn route(action_type: &str) -> Route {
    match action_type {
        "track_reaction_set"
        | "track_reaction_remove"
        | "comment_delete"
        | "comment_like"
        | "comment_unlike" => Route::Mobile,
        "follow_user" | "unfollow_user" | "like_track" | "unlike_track" | "playlist_create"
        | "playlist_delete" => Route::MobileThenApiv1,
        _ => Route::Apiv1,
    }
}

pub(super) enum Absence {
    NotLinked,
    RelinkRequired,
}

pub(super) enum Opened {
    Ready(Box<Linked>),
    Absent(Absence),
    Later(i64),
}

pub(super) enum Refusal {
    Captcha,
    RelinkRequired,
}

pub(super) enum Failure {
    Held(WriteHold),
    Refused(Refusal),
    Later(i64),
    Error(ActionError),
}

enum Renewal {
    Renewed,
    Gone(Absence),
    Later(i64),
}

pub(super) struct Linked {
    session: Session,
    token: Secret<String>,
    client: Arc<MobileClient>,
}

impl Linked {
    pub(super) fn oauth_app_id(&self) -> Uuid {
        self.session.oauth_app_id
    }
}

pub struct MobileSender {
    pool: PgPool,
    refresh: MobileRefresh,
    proxy: Option<Url>,
    clients: Mutex<HashMap<String, Arc<MobileClient>>>,
}

impl MobileSender {
    pub fn new(pool: PgPool, proxy: Option<Url>) -> Self {
        Self {
            refresh: MobileRefresh::new(pool.clone(), proxy.clone()),
            pool,
            proxy,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub(super) async fn open(&self, user_id: &str) -> Result<Opened, sqlx::Error> {
        let Some(session) = session::load(&self.pool, user_id).await? else {
            return Ok(Opened::Absent(Absence::NotLinked));
        };
        if session.status == RELINK_REQUIRED {
            return Ok(Opened::Absent(Absence::RelinkRequired));
        }
        let Some(client) = self.client(&session.app) else {
            return Ok(Opened::Absent(Absence::NotLinked));
        };
        let due = session.expires_at
            <= chrono::Utc::now() + chrono::TimeDelta::seconds(REFRESH_DUE_SECONDS);
        let mut linked = Linked {
            token: session.access_token.clone(),
            session,
            client,
        };
        if due {
            match self.renew(&mut linked, None).await? {
                Renewal::Renewed => {}
                Renewal::Gone(absence) => return Ok(Opened::Absent(absence)),
                Renewal::Later(seconds) => return Ok(Opened::Later(seconds)),
            }
        }
        Ok(Opened::Ready(Box::new(linked)))
    }

    pub(super) async fn send(
        &self,
        linked: &mut Linked,
        op: &'static str,
        request: &MobileRequest,
        gate: &WriteGate<'_>,
    ) -> Result<MobileReply, Failure> {
        gate.check().map_err(|error| match error {
            SoundCloudError::Paused(hold) => Failure::Held(hold),
            error => Failure::Error(ActionError::SoundCloud(error)),
        })?;
        let reply = self.once(linked, op, request).await?;
        if reply.status != UNAUTHORIZED {
            return judged(reply);
        }
        let rejected = token_fingerprint(linked.token.expose());
        match self.renew(linked, Some(rejected)).await.map_err(database)? {
            Renewal::Renewed => {}
            Renewal::Gone(_) => return Err(Failure::Refused(Refusal::RelinkRequired)),
            Renewal::Later(seconds) => return Err(Failure::Later(seconds)),
        }
        let reply = self.once(linked, op, request).await?;
        if reply.status == UNAUTHORIZED {
            session::require_relink(&self.pool, &linked.session.account, linked.token.expose())
                .await
                .map_err(database)?;
            return Err(Failure::Refused(Refusal::RelinkRequired));
        }
        judged(reply)
    }

    async fn once(
        &self,
        linked: &mut Linked,
        op: &'static str,
        request: &MobileRequest,
    ) -> Result<MobileReply, Failure> {
        let caller = Caller::user(&linked.session.device_id, linked.token.clone());
        let answer = linked.client.call(&caller, request).await;
        let class = match &answer {
            Ok(reply) => reply.status_class(),
            Err(error) => error.status_class(),
        };
        crate::metrics::record_mobile_request(op, class);
        let reply = answer.map_err(|error| {
            Failure::Error(ActionError::SoundCloud(SoundCloudError::Mobile(
                error.to_string(),
            )))
        })?;
        if let Some(plan) = &reply.plan {
            self.note_plan(linked, plan.as_str()).await;
        }
        Ok(reply)
    }

    async fn note_plan(&self, linked: &mut Linked, seen: &str) {
        if linked.session.plan.as_deref() == Some(seen) {
            return;
        }
        linked.session.plan = Some(seen.to_owned());
        if let Err(error) = session::note_plan(&self.pool, &linked.session.account, seen).await {
            warn!(%error, "the mobile plan was not kept");
        }
    }

    async fn renew(
        &self,
        linked: &mut Linked,
        rejected: Option<String>,
    ) -> Result<Renewal, sqlx::Error> {
        let answer = self
            .refresh
            .refresh(linked.session.connection_id, rejected.as_deref())
            .await?;
        let token = answer.state.and_then(|state| state.access_token);
        Ok(match (answer.outcome, token) {
            (RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh, Some(token)) => {
                linked.token = Secret::new(token);
                Renewal::Renewed
            }
            (RefreshOutcome::ReauthorizationRequired, _) => Renewal::Gone(Absence::RelinkRequired),
            (RefreshOutcome::NotConnected, _) => Renewal::Gone(Absence::NotLinked),
            _ => Renewal::Later(answer.retry_after_sec.unwrap_or(RENEWAL_RETRY_SECONDS)),
        })
    }

    fn client(&self, app: &MobileApp) -> Option<Arc<MobileClient>> {
        let key = format!("{}\n{}\n{}", app.client_id, app.app_version, app.user_agent);
        let mut clients = guard(&self.clients);
        if let Some(client) = clients.get(&key) {
            return Some(client.clone());
        }
        let client = MobileClient::new(app.clone(), self.egress())
            .inspect_err(|error| warn!(%error, "the mobile client was not built"))
            .ok()?;
        let client = Arc::new(client);
        clients.insert(key, client.clone());
        Some(client)
    }

    fn egress(&self) -> Egress {
        match &self.proxy {
            Some(proxy) => Egress::ProxyThenDirect(proxy.clone()),
            None => Egress::Direct,
        }
    }
}

fn judged(reply: MobileReply) -> Result<MobileReply, Failure> {
    if reply.is_captcha() {
        return Err(Failure::Refused(Refusal::Captcha));
    }
    Ok(reply)
}

fn database(error: sqlx::Error) -> Failure {
    Failure::Error(ActionError::Database(error))
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
