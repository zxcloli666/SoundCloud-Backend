use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::create;
use crate::modules::chat::conversations::fixtures::{dm, group, key_epoch};
use crate::modules::chat::media::support::{bench, call};
use crate::modules::chat::model::MemberState;
use crate::modules::chat::public_chat::support::bench as public_bench;
use crate::modules::chat::{ChatState, b64, rt_keys};
use crate::modules::subscriptions::SubscriptionsService;

pub const BLOCK: i64 = 4096;
const FAR: i64 = 4_000_000_000;

pub struct Bench {
    pub chat: Arc<ChatState>,
    pub pg: PgPool,
}

pub fn someone() -> String {
    format!("7{:011}", Uuid::now_v7().as_u128() % 100_000_000_000)
}

pub fn preset(id: &str) -> Value {
    json!({"kind": "preset", "id": id})
}

pub fn image(media: Uuid) -> Value {
    json!({"kind": "image", "mediaId": media})
}

pub fn chosen(mut backdrop: Value, by: &str) -> Value {
    backdrop["by"] = json!(by);
    backdrop
}

pub fn missing(media: Uuid) -> (StatusCode, Value) {
    (
        StatusCode::CONFLICT,
        json!({"error": "chat_media_missing", "ids": [media]}),
    )
}

pub fn refused(status: StatusCode, code: &str) -> (StatusCode, Value) {
    (status, json!({"error": code}))
}

pub fn report(conversation: Uuid, media: Uuid) -> Value {
    json!({"conversation_id": conversation, "kind": "backdrop", "media_id": media, "reason": "harassment", "comment": " ugly "})
}

impl Bench {
    pub fn new(pg: PgPool) -> Self {
        Self {
            chat: bench(&pg),
            pg,
        }
    }

    pub fn public(pg: PgPool) -> Self {
        Self {
            chat: public_bench(&pg),
            pg,
        }
    }

    pub async fn star(&self, user: &str) -> anyhow::Result<()> {
        SubscriptionsService::new(self.pg.clone(), false)
            .upsert(user, FAR, None)
            .await?;
        Ok(())
    }

    pub async fn unstar(&self, user: &str) -> anyhow::Result<()> {
        SubscriptionsService::new(self.pg.clone(), false)
            .upsert(user, 1, None)
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
        let (status, body) = call(&self.chat, user, method, uri, body).await;
        Ok((StatusCode::from_u16(status)?, body))
    }

    pub async fn put(
        &self,
        user: &str,
        conversation: Uuid,
        body: Value,
    ) -> anyhow::Result<(StatusCode, Value)> {
        self.ask(
            user,
            "PUT",
            &format!("/chat/conversations/{conversation}/backdrop"),
            Some(body),
        )
        .await
    }

    pub async fn show(&self, user: &str, conversation: Uuid) -> anyhow::Result<Value> {
        let (status, view) = self
            .ask(
                user,
                "GET",
                &format!("/chat/conversations/{conversation}"),
                None,
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "{view}");
        Ok(view)
    }

    pub async fn listed(&self, user: &str, conversation: Uuid) -> anyhow::Result<Value> {
        let (status, page) = self.ask(user, "GET", "/chat/conversations", None).await?;
        assert_eq!(status, StatusCode::OK, "{page}");
        let items = page["items"].as_array().cloned().unwrap_or_default();
        Ok(items
            .into_iter()
            .find(|item| item["id"] == json!(conversation))
            .unwrap_or(Value::Null))
    }

    pub async fn file(&self, user: &str, body: Value) -> anyhow::Result<(StatusCode, Value)> {
        self.ask(user, "POST", "/chat/reports", Some(body)).await
    }

    pub async fn send_meta(
        &self,
        user: &str,
        conversation: Uuid,
        media: &[Uuid],
    ) -> anyhow::Result<(StatusCode, Value)> {
        let epoch = key_epoch(&self.pg, conversation).await;
        self.ask(
            user,
            "POST",
            &format!("/chat/conversations/{conversation}/messages"),
            Some(json!({
                "client_id": Uuid::now_v7(),
                "class": "meta",
                "epoch": epoch,
                "body": b64::encode(&vec![3_u8; 300]),
                "media": media
            })),
        )
        .await
    }

    pub async fn dm(&self, user: &str, peer: &str) -> Uuid {
        dm(&self.pg, user, peer, MemberState::Active).await
    }

    pub async fn request(&self, user: &str, peer: &str) -> Uuid {
        dm(&self.pg, user, peer, MemberState::Invited).await
    }

    pub async fn group(
        &self,
        owner: &str,
        admins: &[&str],
        members: &[&str],
    ) -> anyhow::Result<Uuid> {
        let placed: Vec<(&str, MemberState)> = admins
            .iter()
            .chain(members)
            .map(|user| (*user, MemberState::Active))
            .collect();
        let id = group(&self.pg, owner, &placed).await;
        for admin in admins {
            sqlx::query(
                "UPDATE chat.members SET role = 'admin' WHERE conversation_id = $1 AND user_id = $2",
            )
            .bind(id)
            .bind(admin)
            .execute(&self.pg)
            .await?;
        }
        Ok(id)
    }

    pub async fn saved(&self, user: &str) -> anyhow::Result<Uuid> {
        Ok(create::saved(&self.pg, user)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
            .id)
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

    pub async fn stored(&self, conversation: Uuid) -> anyhow::Result<Value> {
        let settings: Value =
            sqlx::query_scalar("SELECT settings FROM chat.conversations WHERE id = $1")
                .bind(conversation)
                .fetch_one(&self.pg)
                .await?;
        Ok(settings.get("backdrop").cloned().unwrap_or(Value::Null))
    }

    pub async fn meta_of(&self, conversation: Uuid) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(
            sqlx::query_scalar("SELECT meta_ct FROM chat.conversations WHERE id = $1")
                .bind(conversation)
                .fetch_one(&self.pg)
                .await?,
        )
    }

    pub async fn holds_of(&self, media: Uuid) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM chat.media_holds WHERE media_id = $1")
                .bind(media)
                .fetch_one(&self.pg)
                .await?,
        )
    }

    pub async fn forget_limits(&self, conversation: Uuid) -> anyhow::Result<()> {
        let mut conn = self.chat.redis.get().await?;
        let _: i64 = redis::cmd("DEL")
            .arg(rt_keys::limit("backdrop:10m", &conversation.to_string()))
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}
