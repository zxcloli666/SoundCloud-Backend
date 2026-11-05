use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::media::support::{AVATAR, IMAGE, THUMB};

#[derive(Clone, Copy)]
pub struct Seed<'a> {
    pub owner: &'a str,
    pub origin: Uuid,
    pub kind: &'a str,
    pub status: &'a str,
    pub store: i16,
}

impl<'a> Seed<'a> {
    pub fn image(owner: &'a str, origin: Uuid, status: &'a str) -> Self {
        Self {
            owner,
            origin,
            kind: "image",
            status,
            store: 1,
        }
    }
}

pub async fn seed(pool: &PgPool, seed: Seed<'_>) -> (Uuid, Option<Uuid>) {
    let root = Uuid::new_v4();
    let bytes = if seed.kind == "image" { IMAGE } else { AVATAR };
    insert_row(pool, &seed, root, seed.kind, None, bytes).await;
    let thumb = if seed.kind == "image" {
        let thumb = Uuid::new_v4();
        insert_row(pool, &seed, thumb, "thumb", Some(root), THUMB).await;
        Some(thumb)
    } else {
        None
    };
    if matches!(seed.status, "uploaded" | "attached" | "reserved") {
        let total = i64::try_from(bytes + thumb.map_or(0, |_| THUMB)).expect("fits");
        let (charged, reserved, files) = if seed.status == "reserved" {
            (0, total, 0)
        } else {
            (total, 0, 1)
        };
        sqlx::query(
            "INSERT INTO chat.usage (user_id, bytes, reserved, files) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id) DO UPDATE SET bytes = chat.usage.bytes + $2,
                 reserved = chat.usage.reserved + $3, files = chat.usage.files + $4",
        )
        .bind(seed.owner)
        .bind(charged)
        .bind(reserved)
        .bind(files)
        .execute(pool)
        .await
        .expect("a usage row");
    }
    (root, thumb)
}

async fn insert_row(
    pool: &PgPool,
    seed: &Seed<'_>,
    id: Uuid,
    kind: &str,
    parent: Option<Uuid>,
    bytes: u64,
) {
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, parent_id, bytes, status, delete_after)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now() + interval '1 hour')",
    )
    .bind(id)
    .bind(seed.store)
    .bind(format!("m/{}", Uuid::new_v4().simple()))
    .bind(seed.owner)
    .bind(seed.origin)
    .bind(kind)
    .bind(parent)
    .bind(i64::try_from(bytes).expect("fits"))
    .bind(seed.status)
    .execute(pool)
    .await
    .expect("a media row");
}

pub async fn add_ref(pool: &PgPool, media: Uuid, conversation: Uuid, seq: i64) {
    sqlx::query("INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, $3)")
        .bind(media)
        .bind(conversation)
        .bind(seq)
        .execute(pool)
        .await
        .expect("a ref");
}

pub async fn usage(pool: &PgPool, user: &str) -> (i64, i64, i32) {
    sqlx::query_as("SELECT bytes, reserved, files FROM chat.usage WHERE user_id = $1")
        .bind(user)
        .fetch_optional(pool)
        .await
        .expect("a usage read")
        .unwrap_or((0, 0, 0))
}

pub async fn status(pool: &PgPool, id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM chat.media WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .expect("a status read")
}

pub async fn refs(pool: &PgPool, media: Uuid) -> Vec<(Uuid, i64)> {
    sqlx::query_as(
        "SELECT conversation_id, seq FROM chat.media_refs WHERE media_id = $1 ORDER BY conversation_id, seq",
    )
    .bind(media)
    .fetch_all(pool)
    .await
    .expect("refs")
}

pub async fn room_conversation(pool: &PgPool, creator: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO chat.conversations (id, kind, created_by) VALUES ($1, 'room', $2)")
        .bind(id)
        .bind(creator)
        .execute(pool)
        .await
        .expect("a room conversation");
    id
}

pub async fn set_usage(pool: &PgPool, user: &str, bytes: i64, day_bytes: i64) {
    sqlx::query(
        "INSERT INTO chat.usage (user_id, bytes, day_bytes, day) VALUES ($1, $2, $3, current_date)
         ON CONFLICT (user_id) DO UPDATE SET bytes = $2, day_bytes = $3, day = current_date",
    )
    .bind(user)
    .bind(bytes)
    .bind(day_bytes)
    .execute(pool)
    .await
    .expect("a usage row");
}
