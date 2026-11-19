mod refresh;
mod send;
mod track;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use redact::Secret;
use sc_mobile::{Egress, MobileApp, MobileClient, MobileReply, UserPlan, streams};
use serde::Deserialize;
use tokio::sync::OnceCell;
use tracing::warn;
use url::Url;

use super::hls;
use super::quality;
use super::restricted::RestrictedSource;
use super::sources::Fetched;
use crate::db::postgres::PgPool;
use crate::db::postgres::mobile::MobileSession;
use track::{Picked, Track};

pub(crate) use refresh::{TokenRefresh, token_refresh};

const MISSED: &str = "missed";
const PREVIEW: &str = "preview";
const REFUSED: &str = "refused";
const CAPTCHA: &str = "captcha";
const ERROR: &str = "error";
const TRANSCODINGS: &str = "transcodings";
const MEDIA: &str = "media";

pub struct Mobile {
    refresh: TokenRefresh,
    http: wreq::Client,
    proxy_url: String,
    direct_fallback: bool,
    decrypts: bool,
    clients: Mutex<HashMap<String, Arc<MobileClient>>>,
}

pub(crate) struct Link {
    session: MobileSession,
    client: Arc<MobileClient>,
    token: Mutex<Secret<String>>,
    fresh: AtomicBool,
    closed: AtomicBool,
    plan: Mutex<Option<String>>,
    unsaved_plan: Mutex<Option<String>>,
    track: OnceCell<Result<Track, &'static str>>,
}

pub(crate) enum Found {
    Clear(Fetched),
    Encrypted(RestrictedSource),
    Miss(&'static str),
}

#[derive(Deserialize)]
struct License {
    url: String,
    #[serde(rename = "licenseAuthToken")]
    license_auth_token: Option<String>,
}

impl Mobile {
    pub(crate) fn new(
        refresh: TokenRefresh,
        http: wreq::Client,
        proxy_url: String,
        direct_fallback: bool,
        decrypts: bool,
    ) -> Self {
        Self {
            refresh,
            http,
            proxy_url,
            direct_fallback,
            decrypts,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn link(&self, session: MobileSession) -> Option<Link> {
        let client = self.client(&session.app)?;
        Some(Link {
            token: Mutex::new(session.access_token.clone()),
            fresh: AtomicBool::new(session.fresh),
            closed: AtomicBool::new(false),
            plan: Mutex::new(session.plan.clone()),
            unsaved_plan: Mutex::new(None),
            track: OnceCell::new(),
            session,
            client,
        })
    }

    pub(crate) async fn find(&self, link: &Link, track_urn: &str, tier: &'static str) -> Found {
        match self.lookup(link, track_urn, tier).await {
            Ok(found) => found,
            Err(outcome) => {
                if outcome != MISSED {
                    link.close();
                }
                Found::Miss(outcome)
            }
        }
    }

    async fn lookup(
        &self,
        link: &Link,
        track_urn: &str,
        tier: &'static str,
    ) -> Result<Found, &'static str> {
        let track = link
            .track(self, track_urn)
            .await
            .inspect_err(|_| link.close())?;
        track.playable_with(link.plan().as_deref())?;
        let picked = track.pick(tier, self.decrypts)?;
        let url = Url::parse(picked.url).map_err(|_| MISSED)?;
        let reply = link.send(self, MEDIA, &streams::media(url)).await?;
        if picked.encrypted {
            self.encrypted(&reply, &picked, track).await
        } else {
            self.clear(&reply, &picked, track).await
        }
    }

    async fn clear(
        &self,
        reply: &MobileReply,
        picked: &Picked<'_>,
        track: &Track,
    ) -> Result<Found, &'static str> {
        let Some(url) = streams::stream_url(reply) else {
            let preview = reply.location.as_ref().is_some_and(track::names_a_preview);
            return Err(if preview { PREVIEW } else { MISSED });
        };
        let playlist = self.playlist(&url, track).await?;
        let source = hls::parse_m3u8(&playlist, url.as_str());
        if source.1.is_empty() {
            return Err(MISSED);
        }
        let (data, content_type) = hls::download_segments(
            &self.http,
            &self.proxy_url,
            source,
            picked.mime,
            false,
            None,
        )
        .await
        .map_err(|error| {
            warn!("[mobile] segments failed: {error}");
            ERROR
        })?;
        Ok(Found::Clear((data, content_type, picked.tier)))
    }

    async fn encrypted(
        &self,
        reply: &MobileReply,
        picked: &Picked<'_>,
        track: &Track,
    ) -> Result<Found, &'static str> {
        let license = reply
            .is_success()
            .then(|| serde_json::from_slice::<License>(&reply.body).ok())
            .flatten()
            .ok_or(MISSED)?;
        let token = license.license_auth_token.ok_or(MISSED)?;
        let url = super::target::public_media(&license.url).ok_or(MISSED)?;
        if url.scheme() != "https" {
            return Err(MISSED);
        }
        if track::names_a_preview(&url) {
            return Err(PREVIEW);
        }
        let manifest = self.playlist(&url, track).await?;
        Ok(Found::Encrypted(RestrictedSource {
            manifest,
            token,
            content_type: hls::mime_to_content_type(picked.mime),
            quality: picked.tier,
        }))
    }

    async fn playlist(&self, url: &Url, track: &Track) -> Result<String, &'static str> {
        let text = hls::fetch_m3u8_text(
            &self.http,
            &self.proxy_url,
            url.as_str(),
            HashMap::new(),
            false,
        )
        .await
        .map_err(|error| {
            warn!("[mobile] playlist failed: {error}");
            ERROR
        })?;
        track.lasts(hls::playlist_seconds(&text))?;
        Ok(text)
    }

    fn client(&self, app: &MobileApp) -> Option<Arc<MobileClient>> {
        let key = format!("{}\n{}\n{}", app.client_id, app.app_version, app.user_agent);
        let mut clients = guard(&self.clients);
        if let Some(client) = clients.get(&key) {
            return Some(client.clone());
        }
        let client = MobileClient::new(app.clone(), self.egress())
            .inspect_err(|error| warn!("[mobile] client not built: {error}"))
            .ok()?;
        let client = Arc::new(client);
        clients.insert(key, client.clone());
        Some(client)
    }

    fn egress(&self) -> Egress {
        match Url::parse(&self.proxy_url) {
            Ok(proxy) if self.direct_fallback => Egress::ProxyThenDirect(proxy),
            Ok(proxy) => Egress::Proxy(proxy),
            Err(_) => Egress::Direct,
        }
    }
}

impl Link {
    pub(crate) fn serves(&self, tier: &str) -> bool {
        !self.closed.load(Ordering::Relaxed) && (tier != quality::UQ || self.serves_uq())
    }

    pub(crate) fn serves_uq(&self) -> bool {
        self.plan()
            .is_some_and(|plan| UserPlan::parse(&plan) == UserPlan::GoPlus)
    }

    pub(crate) fn keep_plan(&self, pg: &PgPool) {
        let Some(plan) = guard(&self.unsaved_plan).take() else {
            return;
        };
        let pg = pg.clone();
        let account = self.session.account.clone();
        tokio::spawn(async move {
            if let Err(error) = pg.note_mobile_plan(&account, &plan).await {
                warn!("[mobile] plan not kept: {error}");
            }
        });
    }

    async fn track(&self, mobile: &Mobile, track_urn: &str) -> Result<&Track, &'static str> {
        self.track
            .get_or_init(|| async {
                let reply = self
                    .send(mobile, TRANSCODINGS, &streams::transcodings(&[track_urn]))
                    .await?;
                if !reply.is_success() {
                    return Err(ERROR);
                }
                track::parse(&reply.body, track_urn).ok_or(MISSED)
            })
            .await
            .as_ref()
            .map_err(|outcome| *outcome)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    fn plan(&self) -> Option<String> {
        guard(&self.plan).clone()
    }

    fn note_plan(&self, seen: &UserPlan) {
        let seen = seen.as_str();
        let mut plan = guard(&self.plan);
        if plan.as_deref() == Some(seen) {
            return;
        }
        *plan = Some(seen.to_owned());
        *guard(&self.unsaved_plan) = Some(seen.to_owned());
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests;
