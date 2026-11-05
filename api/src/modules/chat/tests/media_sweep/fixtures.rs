use std::path::PathBuf;
use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::media::ledger;
use crate::modules::chat::s3::S3;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatResult, ChatState};

pub const IMAGE_BYTES: i64 = 8_192;
pub const THUMB_BYTES: i64 = 4_096;

#[derive(Clone, Debug)]
pub struct Upload {
    pub id: Uuid,
    pub thumb: Option<Uuid>,
    pub owner: String,
    pub store: i16,
    pub key: String,
    pub thumb_key: Option<String>,
    pub bytes: i64,
    pub thumb_bytes: i64,
}

impl Upload {
    pub fn total(&self) -> i64 {
        self.bytes + self.thumb_bytes
    }

    pub fn objects(&self) -> Vec<(String, i64)> {
        let mut objects = vec![(self.key.clone(), self.bytes)];
        if let Some(key) = &self.thumb_key {
            objects.push((key.clone(), self.thumb_bytes));
        }
        objects
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub bytes: i64,
    pub reserved: i64,
    pub files: i32,
}

pub fn ok<T>(result: ChatResult<T>) -> anyhow::Result<T> {
    result.map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub fn state_with(pg: PgPool, redis: deadpool_redis::Pool, media: MediaConfig) -> Arc<ChatState> {
    let shared = chat_state(pg, redis, true);
    let mut state = Arc::into_inner(shared).expect("a fresh chat state");
    state.s3 = S3::new(&media);
    state.config.media = media;
    let chat = Arc::new(state);
    chat.rooms.install_chat(&chat);
    chat
}

pub fn keys_dir(secrets: &[(u16, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chat-media-sweep-{}", Uuid::now_v7().simple()));
    std::fs::create_dir_all(&dir).expect("a keys dir");
    for (id, secret) in secrets {
        std::fs::write(dir.join(format!("s3-secret-{id}")), secret).expect("a secret file");
    }
    dir
}

pub fn object_key() -> String {
    format!("m/{}", Uuid::new_v4().simple())
}

pub async fn reserve(
    pg: &PgPool,
    owner: &str,
    origin: Uuid,
    store: i16,
    thumb: bool,
) -> anyhow::Result<Upload> {
    let upload = Upload {
        id: Uuid::new_v4(),
        thumb: thumb.then(Uuid::new_v4),
        owner: owner.to_owned(),
        store,
        key: object_key(),
        thumb_key: thumb.then(object_key),
        bytes: IMAGE_BYTES,
        thumb_bytes: if thumb { THUMB_BYTES } else { 0 },
    };
    let mut tx = pg.begin().await?;
    sqlx::query("INSERT INTO chat.usage (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    let charged = sqlx::query(
        "UPDATE chat.usage SET reserved = reserved + $2
         WHERE user_id = $1 AND bytes + reserved + $2 <= 1073741824",
    )
    .bind(owner)
    .bind(upload.total())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    anyhow::ensure!(charged == 1, "over the quota");
    sqlx::query(
        "INSERT INTO chat.media
            (id, store, object_key, owner_user, origin_conversation, kind, upload_id, bytes, status, delete_after)
         VALUES ($1, $2, $3, $4, $5, $6, gen_random_uuid(), $7, 'reserved', now() + interval '1 hour')",
    )
    .bind(upload.id)
    .bind(store)
    .bind(&upload.key)
    .bind(owner)
    .bind(origin)
    .bind(if thumb { "image" } else { "avatar" })
    .bind(upload.bytes)
    .execute(&mut *tx)
    .await?;
    if let (Some(id), Some(key)) = (upload.thumb, &upload.thumb_key) {
        sqlx::query(
            "INSERT INTO chat.media
                (id, store, object_key, owner_user, origin_conversation, kind, parent_id, bytes, status, delete_after)
             VALUES ($1, $2, $3, $4, $5, 'thumb', $6, $7, 'reserved', now() + interval '1 hour')",
        )
        .bind(id)
        .bind(store)
        .bind(key)
        .bind(owner)
        .bind(origin)
        .bind(upload.id)
        .bind(upload.thumb_bytes)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(upload)
}

pub async fn commit(pg: &PgPool, upload: &Upload) -> anyhow::Result<bool> {
    let moved: i64 = sqlx::query_scalar(
        "WITH moved AS (
             UPDATE chat.media SET status = 'uploaded', delete_after = now() + interval '1 hour'
             WHERE (id = $1 OR parent_id = $1) AND status = 'reserved'
             RETURNING bytes
         ),
         total AS (SELECT coalesce(sum(bytes), 0)::bigint AS bytes, count(*) AS rows FROM moved),
         charged AS (
             UPDATE chat.usage AS usage
             SET reserved = usage.reserved - total.bytes, bytes = usage.bytes + total.bytes, files = usage.files + 1
             FROM total
             WHERE usage.user_id = $2 AND total.rows > 0
             RETURNING usage.user_id
         )
         SELECT rows FROM total",
    )
    .bind(upload.id)
    .bind(&upload.owner)
    .fetch_one(pg)
    .await?;
    Ok(moved > 0)
}

pub async fn upload(pg: &PgPool, owner: &str, origin: Uuid, store: i16) -> anyhow::Result<Upload> {
    let upload = reserve(pg, owner, origin, store, true).await?;
    anyhow::ensure!(commit(pg, &upload).await?, "the upload did not commit");
    Ok(upload)
}

pub async fn attach(pg: &PgPool, root: Uuid, conversation: Uuid, seq: i64) -> anyhow::Result<bool> {
    let mut tx = pg.begin().await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM chat.media WHERE id = $1 FOR SHARE")
            .bind(root)
            .fetch_optional(&mut *tx)
            .await?;
    if !matches!(status.as_deref(), Some("uploaded" | "attached")) {
        tx.rollback().await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(root)
    .bind(conversation)
    .bind(seq)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE chat.media SET status = 'attached', delete_after = NULL
         WHERE (id = $1 OR parent_id = $1) AND status = 'uploaded'",
    )
    .bind(root)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn redact(pg: &PgPool, conversation: Uuid, seq: i64) -> anyhow::Result<u64> {
    let mut tx = pg.begin().await?;
    let roots: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM chat.media_refs WHERE conversation_id = $1 AND seq = $2 RETURNING media_id",
    )
    .bind(conversation)
    .bind(seq)
    .fetch_all(&mut *tx)
    .await?;
    let settled = ok(ledger::settle(&mut tx, &roots).await)?;
    tx.commit().await?;
    Ok(settled)
}

pub async fn owner_delete(pg: &PgPool, root: Uuid, window_secs: i64) -> anyhow::Result<bool> {
    let moved: i64 = sqlx::query_scalar(
        "WITH roots AS (
             SELECT id, owner_user, status AS was FROM chat.media
             WHERE id = $1 AND kind <> 'thumb' AND status IN ('reserved', 'uploaded', 'attached', 'lost')
             FOR UPDATE
         ),
         moved AS (
             UPDATE chat.media AS media
             SET status = 'deleting', delete_after = NULL,
                 restorable_until = CASE WHEN roots.was IN ('uploaded', 'attached')
                                         THEN now() + $2 * interval '1 second' END
             FROM roots
             WHERE (media.id = roots.id OR media.parent_id = roots.id) AND media.status = roots.was
             RETURNING media.bytes, roots.owner_user, roots.was
         ),
         released AS (
             SELECT owner_user,
                    coalesce(sum(bytes) FILTER (WHERE was = 'reserved'), 0)::bigint AS reserved,
                    coalesce(sum(bytes) FILTER (WHERE was IN ('uploaded', 'attached')), 0)::bigint AS bytes,
                    (count(*) FILTER (WHERE was IN ('uploaded', 'attached')) > 0)::int AS files
             FROM moved GROUP BY owner_user
         ),
         charged AS (
             UPDATE chat.usage AS usage
             SET reserved = usage.reserved - released.reserved,
                 bytes = usage.bytes - released.bytes,
                 files = usage.files - released.files
             FROM released WHERE usage.user_id = released.owner_user
             RETURNING usage.user_id
         )
         SELECT count(*) FROM moved",
    )
    .bind(root)
    .bind(window_secs)
    .fetch_one(pg)
    .await?;
    Ok(moved > 0)
}

pub async fn report(pg: &PgPool, conversation: Uuid, roots: &[Uuid]) -> anyhow::Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO chat.reports
            (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight)
         VALUES ($1, 'reporter', 'target', $2, 1, 1, 'spam', 1)",
    )
    .bind(id)
    .bind(conversation)
    .execute(pg)
    .await?;
    sqlx::query(
        "INSERT INTO chat.media_holds (media_id, report_id, until)
         SELECT unnest($1::uuid[]), $2, now() + interval '30 days'",
    )
    .bind(roots)
    .bind(id)
    .execute(pg)
    .await?;
    Ok(id)
}

pub async fn handle(pg: &PgPool, report: Uuid) -> anyhow::Result<()> {
    sqlx::query("UPDATE chat.reports SET state = 'actioned', handled_at = now() WHERE id = $1")
        .bind(report)
        .execute(pg)
        .await?;
    Ok(())
}

pub async fn queue_trim(pg: &PgPool, conversation: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO chat.media_trim (conversation_id) VALUES ($1)
         ON CONFLICT (conversation_id) DO UPDATE SET queued_at = clock_timestamp()",
    )
    .bind(conversation)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn hide(pg: &PgPool, conversation: Uuid, user: &str, below: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE chat.members SET hidden_before_seq = $3 WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation)
    .bind(user)
    .bind(below)
    .execute(pg)
    .await?;
    queue_trim(pg, conversation).await
}

pub async fn age(pg: &PgPool, root: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE chat.media SET delete_after = now() - interval '1 second'
         WHERE (id = $1 OR parent_id = $1) AND status IN ('reserved', 'uploaded')",
    )
    .bind(root)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn pass_window(pg: &PgPool, root: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE chat.media SET restorable_until = now() - interval '1 second'
         WHERE (id = $1 OR parent_id = $1) AND restorable_until IS NOT NULL",
    )
    .bind(root)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn status(pg: &PgPool, id: Uuid) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT status FROM chat.media WHERE id = $1")
            .bind(id)
            .fetch_optional(pg)
            .await?,
    )
}

pub async fn statuses(pg: &PgPool, upload: &Upload) -> anyhow::Result<Vec<Option<String>>> {
    let mut all = vec![status(pg, upload.id).await?];
    if let Some(thumb) = upload.thumb {
        all.push(status(pg, thumb).await?);
    }
    Ok(all)
}

pub async fn refs(pg: &PgPool, root: Uuid) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM chat.media_refs WHERE media_id = $1")
            .bind(root)
            .fetch_one(pg)
            .await?,
    )
}

pub async fn usage(pg: &PgPool, owner: &str) -> anyhow::Result<Usage> {
    let row: Option<(i64, i64, i32)> =
        sqlx::query_as("SELECT bytes, reserved, files FROM chat.usage WHERE user_id = $1")
            .bind(owner)
            .fetch_optional(pg)
            .await?;
    let (bytes, reserved, files) = row.unwrap_or_default();
    Ok(Usage {
        bytes,
        reserved,
        files,
    })
}

pub async fn drift(pg: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(sum(abs(usage.bytes - coalesce(live.bytes, 0))
                         + abs(usage.reserved - coalesce(live.reserved, 0))
                         + abs(usage.files - coalesce(live.files, 0))), 0)::bigint
         FROM chat.usage AS usage
         LEFT JOIN (
             SELECT owner_user,
                    sum(bytes) FILTER (WHERE status IN ('uploaded', 'attached')) AS bytes,
                    sum(bytes) FILTER (WHERE status = 'reserved') AS reserved,
                    count(*) FILTER (WHERE status IN ('uploaded', 'attached') AND kind <> 'thumb') AS files
             FROM chat.media GROUP BY owner_user
         ) AS live ON live.owner_user = usage.user_id",
    )
    .fetch_one(pg)
    .await?)
}

pub async fn rows_of(pg: &PgPool, origin: Uuid) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM chat.media WHERE origin_conversation = $1")
            .bind(origin)
            .fetch_one(pg)
            .await?,
    )
}

pub async fn snapshot(pg: &PgPool) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT concat_ws('|',
             (SELECT string_agg(concat_ws(',', id, store, status, lease_until, attempts, restorable_until, delete_after), ';' ORDER BY id) FROM chat.media),
             (SELECT string_agg(concat_ws(',', media_id, conversation_id, seq), ';' ORDER BY media_id, conversation_id, seq) FROM chat.media_refs),
             (SELECT string_agg(concat_ws(',', media_id, report_id), ';' ORDER BY media_id, report_id) FROM chat.media_holds),
             (SELECT string_agg(conversation_id::text, ';' ORDER BY conversation_id) FROM chat.media_trim),
             (SELECT string_agg(concat_ws(',', user_id, bytes, reserved, files), ';' ORDER BY user_id) FROM chat.usage))",
    )
    .fetch_one(pg)
    .await?)
}
