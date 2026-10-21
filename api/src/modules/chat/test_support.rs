use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use chat_wire::ock::OckSecret;
use serde_json::Value;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::modules::auth::{AuthHealthService, AuthService, LinkService};
use crate::modules::chat::caller::{Callers, ChatCaller};
use crate::modules::chat::channel::ock::OckRing;
use crate::modules::chat::config::{Access, ModerationConfig};
use crate::modules::chat::devices::DeviceCache;
use crate::modules::chat::hot_cache::{HOT_CACHE_BYTES, HotCache};
use crate::modules::chat::hub::ChatHub;
use crate::modules::chat::kek::KekRing;
use crate::modules::chat::model::Tier;
use crate::modules::chat::moderation::Moderator;
use crate::modules::chat::moderation::client::AiClient;
use crate::modules::chat::{ChatConfig, ChatState};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::modules::rooms::{RoomHub, RoomsService};
use crate::sc::ScClient;

pub fn request_vector(name: &str) -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../utils/chat-wire/vectors/request.json"
    );
    let vectors: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("chat-wire vectors"))
            .expect("vector json");
    vectors["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|case| case["name"] == name)
        .cloned()
        .expect("a vector case with that name")
}

pub fn hex_field(case: &Value, field: &str) -> Vec<u8> {
    hex::decode(case[field].as_str().expect("hex field")).expect("valid hex")
}

pub fn vector_ock() -> OckSecret {
    OckSecret::from_bytes(&hex_field(&request_vector("sync"), "ock_secret")).expect("test ock")
}

pub fn test_keks() -> KekRing {
    KekRing::new(
        1,
        1,
        BTreeMap::from([(1, [0x4b; 32])]),
        BTreeMap::from([(1, [0x52; 32])]),
    )
    .expect("a kek ring")
}

pub fn config(enabled: bool) -> ChatConfig {
    ChatConfig {
        enabled,
        access: Access::Beta,
        beta_users: HashSet::new(),
        send_paused: false,
        pg_pool_max: 2,
        max_parked: 100,
        hold_max_ms: 20_000,
        keys_dir: PathBuf::from("/nonexistent"),
        ock_current: 1,
        kek_current: 1,
        reports_kek_current: 1,
        loadtest: false,
        moderation: ModerationConfig {
            enabled: false,
            url: None,
            models: Vec::new(),
        },
    }
}

pub fn chat_state(pg: PgPool, redis: deadpool_redis::Pool, enabled: bool) -> Arc<ChatState> {
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: String::new(),
        proxy_fallback: false,
        api_base: Some("http://127.0.0.1:1".to_owned()),
        home_base: Some("http://127.0.0.1:1".to_owned()),
    })
    .expect("an sc client");
    let auth = AuthService::new(
        pg.clone(),
        sc,
        OAuthAppsService::new(pg.clone()),
        AuthHealthService::with_database(redis.clone(), pg.clone()),
    );
    let config = config(enabled);
    Arc::new(ChatState {
        ocks: OckRing::new(1, BTreeMap::from([(1, vector_ock())])).expect("a ring"),
        keks: test_keks(),
        devices: DeviceCache::new(),
        callers: Callers::new(),
        hub: ChatHub::new(),
        hot: HotCache::new(HOT_CACHE_BYTES),
        moderator: Moderator::new(
            AiClient::load(&config.moderation, &config.keys_dir).expect("moderation off"),
        ),
        link: LinkService::new(pg.clone(), auth.clone()),
        auth,
        rooms: RoomsService::new(redis.clone(), RoomHub::new(None)),
        shutdown: CancellationToken::new(),
        pg,
        redis,
        config,
    })
}

pub fn offline_chat_state(enabled: bool) -> Arc<ChatState> {
    let pg = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://scd@127.0.0.1:1/none")
        .expect("a lazy pool");
    let redis = crate::redis::connect_url("redis://127.0.0.1:1").expect("a lazy redis pool");
    chat_state(pg, redis, enabled)
}

pub fn caller() -> ChatCaller {
    ChatCaller {
        user_id: "101".to_owned(),
        session_id: Uuid::from_u128(1),
        device: None,
        tier: Tier::Direct,
        max_bytes: 262_144,
    }
}
