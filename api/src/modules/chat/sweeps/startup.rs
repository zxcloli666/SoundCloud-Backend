use std::collections::HashMap;

use uuid::Uuid;

use crate::modules::chat::sweeps::hooks;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const REBUMP_CHUNK: usize = 256;

const ROOM: &str = "room";

const SWEEP_STATEMENTS: [&str; 2] = [
    include_str!("../../../../queries/chat/sweeps/purge_messages.sql"),
    include_str!("../../../../queries/chat/sweeps/expired_delete.sql"),
];

struct Recent {
    id: Uuid,
    kind: String,
    last_seq: i64,
}

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let cancelled = cancel_orphans(chat).await?;
    if cancelled > 0 {
        tracing::warn!(
            cancelled,
            "chat sweep queries left by a previous chat-api were cancelled"
        );
    }
    rebump(chat).await
}

pub async fn cancel_orphans(chat: &ChatState) -> ChatResult<u64> {
    let statements: Vec<String> = SWEEP_STATEMENTS
        .iter()
        .map(|statement| statement.trim().to_owned())
        .collect();
    let cancelled: i64 = sqlx::query_scalar(include_str!(
        "../../../../queries/chat/sweeps/cancel_orphans.sql"
    ))
    .bind(&statements)
    .fetch_one(&chat.pg)
    .await?;
    Ok(u64::try_from(cancelled).unwrap_or(0))
}

pub async fn rebump(chat: &ChatState) -> ChatResult<u64> {
    let recent: Vec<Recent> = sqlx::query_file!("queries/chat/sweeps/recent_conversations.sql")
        .fetch_all(&chat.pg)
        .await?
        .into_iter()
        .map(|row| Recent {
            id: row.id,
            kind: row.kind,
            last_seq: row.last_seq,
        })
        .collect();
    let mut rebumped = 0;
    for chunk in recent.chunks(REBUMP_CHUNK) {
        let lagging = lagging(chat, chunk).await?;
        rebumped += lagging.len() as u64;
        let mut recipients = recipients(chat, &lagging).await?;
        for row in lagging {
            let users = recipients.remove(&row.id).unwrap_or_default();
            hooks::bump(chat, row.id, row.last_seq, &users).await?;
        }
    }
    metrics::counter!("chat_rebump_total").increment(rebumped);
    Ok(rebumped)
}

async fn lagging<'a>(chat: &ChatState, rows: &'a [Recent]) -> ChatResult<Vec<&'a Recent>> {
    let keys: Vec<String> = rows
        .iter()
        .map(|row| rt_keys::conversation_seq(row.id))
        .collect();
    let bumped: Vec<Option<i64>> = {
        let mut conn = chat.redis.get().await?;
        redis::cmd("MGET").arg(&keys).query_async(&mut conn).await?
    };
    Ok(rows
        .iter()
        .zip(bumped)
        .filter(|(row, bumped)| bumped.is_none_or(|seq| seq < row.last_seq))
        .map(|(row, _)| row)
        .collect())
}

async fn recipients(chat: &ChatState, rows: &[&Recent]) -> ChatResult<HashMap<Uuid, Vec<String>>> {
    let mut found: HashMap<Uuid, Vec<String>> = HashMap::new();
    let members_of: Vec<Uuid> = rows
        .iter()
        .filter(|row| row.kind != ROOM)
        .map(|row| row.id)
        .collect();
    if !members_of.is_empty() {
        for row in sqlx::query_file!("queries/chat/sweeps/live_members.sql", &members_of)
            .fetch_all(&chat.pg)
            .await?
        {
            found
                .entry(row.conversation_id)
                .or_default()
                .push(row.user_id);
        }
    }
    for row in rows.iter().filter(|row| row.kind == ROOM) {
        if let Some(room) = hooks::room_of_chat(chat, row.id).await? {
            found.insert(
                row.id,
                room.members
                    .into_iter()
                    .map(|member| member.user_id)
                    .collect(),
            );
        }
    }
    Ok(found)
}
