use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::caller::ChatCaller;
use crate::modules::chat::model::Tier;
use crate::modules::chat::rooms::bridge;
use crate::modules::chat::rt_keys;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatState, routes};
use crate::modules::rooms::model::Profile;
use crate::modules::subscriptions::SubscriptionsService;

pub const BLOCK: i64 = 4096;
const FAR: i64 = 4_000_000_000;

pub struct Bench {
    pub chat: Arc<ChatState>,
    pub router: axum::Router,
    pub pg: PgPool,
}

pub fn someone() -> String {
    format!("7{:011}", Uuid::now_v7().as_u128() % 100_000_000_000)
}

pub fn preset(id: &str) -> Value {
    json!({"backdrop": {"kind": "preset", "id": id}})
}

pub fn image(media: Uuid) -> Value {
    json!({"backdrop": {"kind": "image", "mediaId": media}})
}

pub fn missing(media: Uuid) -> (StatusCode, Value) {
    (
        StatusCode::CONFLICT,
        json!({"error": "chat_media_missing", "ids": [media]}),
    )
}

pub async fn put(
    bench: &Bench,
    user: &str,
    code: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    bench
        .ask(
            user,
            "PUT",
            &format!("/chat/rooms/{code}/settings"),
            Some(body),
        )
        .await
}

pub fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        avatar_url: None,
    }
}

impl Bench {
    pub fn new(pg: PgPool) -> anyhow::Result<Self> {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let chat = chat_state(pg.clone(), crate::redis::connect_url(&url)?, true);
        chat.rooms.install_chat(&chat);
        let router = routes::inner().with_state(chat.clone());
        Ok(Self { chat, router, pg })
    }

    pub async fn star(&self, user: &str) -> anyhow::Result<()> {
        SubscriptionsService::new(self.pg.clone(), false)
            .upsert(user, FAR, None)
            .await?;
        Ok(())
    }

    pub async fn ask(
        &self,
        user: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> anyhow::Result<(StatusCode, Value)> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))?;
        request.extensions_mut().insert(ChatCaller {
            user_id: user.to_owned(),
            session_id: Uuid::now_v7(),
            device: None,
            tier: Tier::Direct,
            max_bytes: 262_144,
        });
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }

    pub async fn room(&self, host: &str, public: bool, guests: &[&str]) -> anyhow::Result<String> {
        let code = self
            .chat
            .rooms
            .create(host, &profile("Host"), public, None)
            .await?
            .code;
        for guest in guests {
            self.chat
                .rooms
                .join(&code, guest, &profile("Guest"), &self.pg)
                .await?;
        }
        Ok(code)
    }

    pub async fn open_chat(&self, code: &str, user: &str) -> anyhow::Result<Uuid> {
        let opened = self.chat.rooms.claim_chat(code, user).await?;
        bridge::ensure_conversation(&self.chat, &opened)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        Ok(opened.conversation_id)
    }

    pub async fn upload(
        &self,
        owner: &str,
        origin: Uuid,
        kind: &str,
        status: &str,
    ) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, bytes, status, delete_after)
             VALUES ($1, 1, $2, $3, $4, $5, $6, $7, now() + interval '1 hour')",
        )
        .bind(id)
        .bind(format!("m/{}", id.simple()))
        .bind(owner)
        .bind(origin)
        .bind(kind)
        .bind(BLOCK)
        .bind(status)
        .execute(&self.pg)
        .await?;
        if status != "reserved" {
            sqlx::query(
                "INSERT INTO chat.usage (user_id, bytes, files) VALUES ($1, $2, 1)
                 ON CONFLICT (user_id) DO UPDATE SET bytes = chat.usage.bytes + $2, files = chat.usage.files + 1",
            )
            .bind(owner)
            .bind(BLOCK)
            .execute(&self.pg)
            .await?;
        }
        Ok(id)
    }

    pub async fn status_of(&self, media: Uuid) -> anyhow::Result<String> {
        Ok(
            sqlx::query_scalar("SELECT status FROM chat.media WHERE id = $1")
                .bind(media)
                .fetch_one(&self.pg)
                .await?,
        )
    }

    pub async fn refs_of(&self, conversation: Uuid) -> anyhow::Result<Vec<(Uuid, i64)>> {
        Ok(sqlx::query_as(
            "SELECT media_id, seq FROM chat.media_refs WHERE conversation_id = $1 ORDER BY media_id",
        )
        .bind(conversation)
        .fetch_all(&self.pg)
        .await?)
    }

    pub async fn usage_of(&self, user: &str) -> anyhow::Result<(i64, i32)> {
        Ok(
            sqlx::query_as("SELECT bytes, files FROM chat.usage WHERE user_id = $1")
                .bind(user)
                .fetch_one(&self.pg)
                .await?,
        )
    }

    pub async fn mark_meta(&self, conversation: Uuid) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE chat.conversations SET meta_ct = '\\x0102'::bytea, meta_epoch = key_epoch WHERE id = $1",
        )
        .bind(conversation)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    pub async fn meta_of(&self, conversation: Uuid) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(
            sqlx::query_scalar("SELECT meta_ct FROM chat.conversations WHERE id = $1")
                .bind(conversation)
                .fetch_one(&self.pg)
                .await?,
        )
    }

    pub async fn forget_limits(&self, key: &str) -> anyhow::Result<()> {
        let mut conn = self.chat.redis.get().await?;
        let _: i64 = redis::cmd("DEL")
            .arg(rt_keys::limit("backdrop:10m", key))
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

pub async fn steal(bench: &Bench, conversation: Uuid, ms: u64) -> anyhow::Result<()> {
    let mut conn = bench.chat.redis.get().await?;
    let _: () = redis::cmd("SET")
        .arg(rt_keys::backdrop_lock(conversation))
        .arg("someone else")
        .arg("PX")
        .arg(ms)
        .query_async(&mut conn)
        .await?;
    Ok(())
}
