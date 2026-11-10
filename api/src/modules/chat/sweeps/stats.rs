use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 60;
pub const MEDIA_KINDS: [&str; 4] = ["image", "thumb", "avatar", "backdrop"];
pub const MEDIA_STATUSES: [&str; 6] = [
    "reserved", "uploaded", "attached", "held", "deleting", "lost",
];

const TABLES: &str = include_str!("../../../../queries/chat/sweeps/stats_tables.sql");

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    media(chat).await?;
    conversations(chat).await?;
    people(chat).await?;
    tables(chat).await?;
    Ok(0)
}

async fn media(chat: &ChatState) -> ChatResult<()> {
    let kinds = MEDIA_KINDS.map(str::to_owned);
    let statuses = MEDIA_STATUSES.map(str::to_owned);
    let rows = sqlx::query_file!(
        "queries/chat/sweeps/stats_media.sql",
        &kinds[..],
        &statuses[..]
    )
    .fetch_all(&chat.pg)
    .await?;
    for row in rows {
        metrics::gauge!("chat_media_objects", "kind" => row.kind.clone(), "status" => row.status.clone())
            .set(row.objects as f64);
        metrics::gauge!("chat_media_bytes", "kind" => row.kind, "status" => row.status)
            .set(row.bytes as f64);
    }
    Ok(())
}

async fn conversations(chat: &ChatState) -> ChatResult<()> {
    let kinds = ConversationKind::ALL.map(|kind| kind.as_str().to_owned());
    let rows = sqlx::query_file!("queries/chat/sweeps/stats_conversations.sql", &kinds[..])
        .fetch_all(&chat.pg)
        .await?;
    for row in rows {
        metrics::gauge!("chat_conversations", "kind" => row.kind.clone()).set(row.chats as f64);
        metrics::gauge!("chat_conversations_active", "kind" => row.kind.clone(), "window" => "1d")
            .set(row.active_day as f64);
        metrics::gauge!("chat_conversations_active", "kind" => row.kind.clone(), "window" => "7d")
            .set(row.active_week as f64);
        metrics::gauge!("chat_members", "kind" => row.kind).set(row.members as f64);
    }
    Ok(())
}

async fn people(chat: &ChatState) -> ChatResult<()> {
    let row = sqlx::query_file!("queries/chat/sweeps/stats_people.sql")
        .fetch_one(&chat.pg)
        .await?;
    metrics::gauge!("chat_users").set(row.users as f64);
    metrics::gauge!("chat_users_new", "window" => "1d").set(row.new_day as f64);
    metrics::gauge!("chat_users_active", "window" => "1d").set(row.active_day as f64);
    metrics::gauge!("chat_users_active", "window" => "7d").set(row.active_week as f64);
    metrics::gauge!("chat_users_restricted").set(row.restricted as f64);
    metrics::gauge!("chat_devices_active").set(row.devices as f64);
    metrics::gauge!("chat_reports_open").set(row.reports_open as f64);
    Ok(())
}

async fn tables(chat: &ChatState) -> ChatResult<()> {
    let found: Vec<(String, i64, i64)> = sqlx::query_as(TABLES).fetch_all(&chat.pg).await?;
    for (table, rows, bytes) in found {
        metrics::gauge!("chat_table_rows", "table" => table.clone()).set(rows as f64);
        metrics::gauge!("chat_table_bytes", "table" => table).set(bytes as f64);
    }
    Ok(())
}
