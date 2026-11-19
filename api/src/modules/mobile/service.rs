use std::sync::Arc;

use redact::Secret;
use sc_mobile::{Egress, MobileApp, MobileClient};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::config::{MobileLinkAllow, SoundcloudCfg};
use crate::error::{AppError, AppResult};
use crate::modules::auth::{AuthService, CoreRpc};

use super::limits::Limits;

pub struct MobileService {
    pub(super) pool: PgPool,
    pub(super) auth: Arc<AuthService>,
    pub(super) core: Arc<CoreRpc>,
    pub(super) allow: MobileLinkAllow,
    pub(super) limits: Limits,
    proxy: Option<Url>,
    proxy_fallback: bool,
}

pub(super) struct Profile {
    pub(super) id: Uuid,
    pub(super) app: MobileApp,
}

struct ProfileRow {
    id: Uuid,
    client_id: String,
    client_secret: String,
    app_id: String,
    app_version: String,
    user_agent: String,
}

impl MobileService {
    pub fn new(
        pool: PgPool,
        auth: Arc<AuthService>,
        core: Arc<CoreRpc>,
        soundcloud: &SoundcloudCfg,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            auth,
            core,
            allow: soundcloud.mobile_link.clone(),
            limits: Limits::default(),
            proxy: Url::parse(soundcloud.proxy_url.trim()).ok(),
            proxy_fallback: soundcloud.proxy_fallback,
        })
    }

    pub(super) async fn picked_profile(&self) -> AppResult<Option<Profile>> {
        let row = sqlx::query_file_as!(ProfileRow, "queries/mobile/pick_app.sql")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(ProfileRow::profile))
    }

    pub(super) async fn profile(&self, oauth_app_id: Uuid) -> AppResult<Option<Profile>> {
        let row = sqlx::query_file_as!(ProfileRow, "queries/mobile/app_by_id.sql", oauth_app_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(ProfileRow::profile))
    }

    pub(super) fn client(&self, app: MobileApp) -> AppResult<MobileClient> {
        let egress = match (&self.proxy, self.proxy_fallback) {
            (Some(proxy), true) => Egress::ProxyThenDirect(proxy.clone()),
            (Some(proxy), false) => Egress::Proxy(proxy.clone()),
            (None, _) => Egress::Direct,
        };
        MobileClient::new(app, egress)
            .map_err(|error| AppError::internal(format!("mobile client: {error}")))
    }
}

impl ProfileRow {
    fn profile(self) -> Profile {
        Profile {
            id: self.id,
            app: MobileApp {
                client_id: self.client_id,
                client_secret: Secret::new(self.client_secret),
                app_id: self.app_id,
                app_version: self.app_version,
                user_agent: self.user_agent,
            },
        }
    }
}
