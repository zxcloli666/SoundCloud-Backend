use std::sync::Arc;

use axum::extract::FromRef;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::common::admission::PublicAdmission;
use crate::modules::auth::{AuthHealthService, AuthService, LinkService};
use crate::modules::chat::ChatState;
use crate::modules::chat::caller::Callers;
use crate::modules::chat::channel::ock::OckRing;
use crate::modules::chat::hot_cache::{HOT_CACHE_BYTES, HotCache};
use crate::modules::chat::hub::ChatHub;
use crate::modules::chat::kek::KekRing;
use crate::modules::chat::moderation::Moderator;
use crate::modules::chat::moderation::client::AiClient;
use crate::modules::oauth_apps::OAuthAppsService;
use crate::modules::rooms::{RoomHub, RoomsService};
use crate::rt::config::RtConfig;
use crate::sc::ScClient;
use crate::{db, redis};

#[derive(Clone, FromRef)]
pub struct RtState {
    pub pg: PgPool,
    pub auth: Arc<AuthService>,
    pub admission: Arc<PublicAdmission>,
    pub rooms: Arc<RoomsService>,
    pub chat: Arc<ChatState>,
}

pub async fn build(config: &RtConfig) -> RtState {
    let pg = db::connect_pool(&config.database, config.database.pool_max)
        .await
        .expect("Failed to connect to PostgreSQL");
    db::verify_schema(&pg)
        .await
        .expect("PostgreSQL schema is incompatible; run jobs-migrate core before chat-api");
    let chat_pg = db::connect_pool(&config.database, config.chat.pg_pool_max)
        .await
        .expect("Failed to connect the chat pool to PostgreSQL");

    let music_redis = redis::connect_url(&config.redis_url).expect("Failed to create Redis pool");
    let redis_rt =
        redis::connect_url(&config.redis_rt_url).expect("Failed to create the redis-rt pool");
    let admission = PublicAdmission::new(
        redis::connect_admission_url(&config.redis_url, config.admission.max_in_flight)
            .expect("Failed to create admission Redis pool"),
        config.admission.clone(),
    );

    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: config.soundcloud.proxy_url.clone(),
        proxy_fallback: config.soundcloud.proxy_fallback,
        api_base: None,
        home_base: None,
    })
    .expect("Failed to build SC HTTP client");
    let auth = AuthService::new(
        pg.clone(),
        sc,
        OAuthAppsService::new(pg.clone()),
        AuthHealthService::with_database(music_redis, pg.clone()),
    );
    let link = LinkService::new(pg.clone(), auth.clone());
    let rooms = RoomsService::new(redis_rt.clone(), RoomHub::new(None));

    let keys_dir = &config.chat.keys_dir;
    let chat = Arc::new(ChatState {
        ocks: OckRing::load(keys_dir, config.chat.ock_current)
            .expect("CHAT_KEYS_DIR must hold the current OCK"),
        keks: KekRing::load(
            keys_dir,
            config.chat.kek_current,
            config.chat.reports_kek_current,
        )
        .expect("CHAT_KEYS_DIR must hold the current KEKs"),
        callers: Callers::new(),
        hub: ChatHub::new(),
        hot: HotCache::new(HOT_CACHE_BYTES),
        moderator: Moderator::new(
            AiClient::load(&config.chat.moderation, keys_dir)
                .expect("chat moderation config is incomplete"),
        ),
        auth: auth.clone(),
        link,
        rooms: rooms.clone(),
        shutdown: CancellationToken::new(),
        config: config.chat.clone(),
        pg: chat_pg,
        redis: redis_rt,
    });

    RtState {
        pg,
        auth,
        admission,
        rooms,
        chat,
    }
}
