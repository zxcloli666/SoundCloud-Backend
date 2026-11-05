use std::path::PathBuf;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::caller::CallerDevice;
use crate::modules::chat::conversations::fixtures::DEVICE;
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::model::Tier;
use crate::modules::chat::routes::inner;
use crate::modules::chat::s3::S3;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatCaller, ChatState};
use crate::modules::subscriptions::SubscriptionsService;

pub const IMAGE: u64 = 151_552;
pub const THUMB: u64 = 24_576;
pub const AVATAR: u64 = 16_384;
pub const FAR: i64 = 4_000_000_000;

pub struct StoreSpec {
    pub id: u16,
    pub bucket: String,
    pub cap_gb: u64,
}

pub struct Rig {
    pub endpoint: String,
    pub presign: String,
    pub access_key: String,
    pub secret: String,
    pub stores: Vec<StoreSpec>,
    pub write: u16,
}

impl Rig {
    pub fn offline() -> Self {
        Self {
            endpoint: "http://127.0.0.1:1".to_owned(),
            presign: "https://s3.scnative.dev".to_owned(),
            access_key: "AKIATEST".to_owned(),
            secret: "not-a-real-secret".to_owned(),
            stores: vec![StoreSpec {
                id: 1,
                bucket: "scd-chat".to_owned(),
                cap_gb: 200,
            }],
            write: 1,
        }
    }

    pub fn config(&self) -> MediaConfig {
        let dir = keys_dir();
        let mut pairs = vec![
            ("CHAT_MEDIA_UPLOADS".to_owned(), "true".to_owned()),
            ("CHAT_STORE_WRITE".to_owned(), self.write.to_string()),
        ];
        let ids: Vec<String> = self
            .stores
            .iter()
            .map(|store| store.id.to_string())
            .collect();
        pairs.push(("CHAT_STORES".to_owned(), ids.join(",")));
        for store in &self.stores {
            let id = store.id;
            std::fs::write(dir.join(format!("s3-secret-{id}")), &self.secret).expect("a secret");
            pairs.push((format!("CHAT_STORE_{id}_ENDPOINT"), self.endpoint.clone()));
            pairs.push((format!("CHAT_STORE_{id}_PRESIGN"), self.presign.clone()));
            pairs.push((format!("CHAT_STORE_{id}_BUCKET"), store.bucket.clone()));
            pairs.push((
                format!("CHAT_STORE_{id}_ACCESS_KEY"),
                self.access_key.clone(),
            ));
            pairs.push((format!("CHAT_STORE_{id}_CAP_GB"), store.cap_gb.to_string()));
        }
        let config = MediaConfig::parse(
            |name| {
                pairs
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
            },
            &dir,
        )
        .expect("a media config");
        std::fs::remove_dir_all(&dir).ok();
        config
    }
}

fn keys_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chat-media-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("a keys dir");
    dir
}

pub fn redis() -> deadpool_redis::Pool {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    crate::redis::connect_url(&url).expect("a redis pool")
}

pub fn bench(pool: &PgPool) -> Arc<ChatState> {
    bench_with(pool, &Rig::offline())
}

pub fn bench_with(pool: &PgPool, rig: &Rig) -> Arc<ChatState> {
    with_media(chat_state(pool.clone(), redis(), true), rig.config())
}

pub fn with_media(mut chat: Arc<ChatState>, media: MediaConfig) -> Arc<ChatState> {
    let state = Arc::get_mut(&mut chat).expect("a fresh state");
    state.s3 = S3::new(&media);
    state.config.media = media;
    chat
}

pub fn user() -> String {
    format!("3{:010}", rand::random::<u32>())
}

pub fn who(user: &str) -> ChatCaller {
    ChatCaller {
        user_id: user.to_owned(),
        session_id: Uuid::now_v7(),
        device: Some(CallerDevice {
            id: DEVICE,
            created_at: chrono::Utc::now() - chrono::Duration::days(30),
            history_unlocked: true,
        }),
        tier: Tier::Direct,
        max_bytes: 262_144,
    }
}

pub async fn star(pool: &PgPool, user: &str) {
    SubscriptionsService::new(pool.clone(), false)
        .upsert(user, FAR, Some(1))
        .await
        .expect("a subscription");
}

pub async fn call(
    chat: &Arc<ChatState>,
    user: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let payload = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let mut request = builder.body(payload).expect("a request");
    request.extensions_mut().insert(who(user));
    let response = inner()
        .with_state(chat.clone())
        .oneshot(request)
        .await
        .expect("an answer");
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub fn error(answer: &(u16, Value)) -> (u16, &str) {
    (answer.0, answer.1["error"].as_str().unwrap_or_default())
}

pub fn image_body(conversation: Uuid) -> Value {
    json!({
        "upload_id": Uuid::new_v4(), "conversation_id": conversation,
        "kind": "image", "bytes": IMAGE, "thumb_bytes": THUMB
    })
}

pub fn avatar_body(conversation: Uuid) -> Value {
    json!({
        "upload_id": Uuid::new_v4(), "conversation_id": conversation,
        "kind": "avatar", "bytes": AVATAR
    })
}

pub async fn reserve(chat: &Arc<ChatState>, user: &str, body: Value) -> Value {
    let answer = call(chat, user, "POST", "/chat/media/reserve", Some(body)).await;
    assert_eq!(answer.0, 200, "{answer:?}");
    answer.1
}

pub fn id_of(answer: &Value, field: &str) -> Uuid {
    serde_json::from_value(answer[field].clone()).expect("an id")
}

pub async fn unstar(pool: &PgPool, user: &str) {
    SubscriptionsService::new(pool.clone(), false)
        .upsert(user, 1, None)
        .await
        .expect("a lapsed subscription");
}

pub struct LiveRoom {
    pub code: String,
    pub conversation: Uuid,
}

pub async fn live_room(chat: &Arc<ChatState>, host: &str, guests: &[&str]) -> LiveRoom {
    chat.rooms.install_chat(chat);
    let profile = crate::modules::rooms::model::Profile {
        name: "Member".to_owned(),
        avatar_url: None,
    };
    let code = chat
        .rooms
        .create(host, &profile, false, None)
        .await
        .expect("a room")
        .code;
    for guest in guests {
        chat.rooms
            .join(&code, guest, &profile, &chat.pg)
            .await
            .expect("a guest");
    }
    let opened = call(chat, host, "GET", &format!("/chat/rooms/{code}/chat"), None).await;
    assert_eq!(opened.0, 200, "{opened:?}");
    LiveRoom {
        code,
        conversation: id_of(&opened.1, "conversation_id"),
    }
}

pub async fn until_blocked(pool: &PgPool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'
             )",
        )
        .fetch_one(pool)
        .await
        .expect("pg_stat_activity");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no session waited on a lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

pub async fn cancel_a_lock_wait(
    pg: &PgPool,
    after: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: Option<(i32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT pid, query_start FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'
               AND ($1::timestamptz IS NULL OR query_start > $1)
             ORDER BY query_start
             LIMIT 1",
        )
        .bind(after)
        .fetch_optional(pg)
        .await?;
        if let Some((pid, started)) = waiting {
            let _: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
                .bind(pid)
                .fetch_one(pg)
                .await?;
            return Ok(started);
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "nothing waited on a lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
