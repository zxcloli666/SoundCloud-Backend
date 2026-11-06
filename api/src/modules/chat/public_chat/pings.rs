use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::MessageRow;
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::sync::bump;
use crate::modules::chat::{ChatResult, ChatState};

const PINGS: &str = "chat_public_pings_total";

pub async fn deliver(chat: &ChatState, row: &MessageRow, asked: &[String], sender: &str) {
    if let Err(error) = run(chat, row, asked, sender).await {
        tracing::warn!(code = error.code(), "public chat pings were not sent");
    }
}

async fn run(chat: &ChatState, row: &MessageRow, asked: &[String], sender: &str) -> ChatResult<()> {
    let candidates: Vec<String> = asked
        .iter()
        .filter(|user| user.as_str() != sender)
        .cloned()
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    let members = sqlx::query_file_scalar!("queries/chat/public/ping_members.sql", ID, &candidates)
        .fetch_all(&chat.pg)
        .await?;
    metrics::counter!(PINGS, "result" => "unknown")
        .increment((candidates.len() - members.len()) as u64);
    let mut allowed = Vec::with_capacity(members.len());
    for user in members {
        if limits::admit(chat, Bucket::PublicPing, &user).await.is_ok() {
            allowed.push(user);
        } else {
            metrics::counter!(PINGS, "result" => "limited").increment(1);
        }
    }
    if allowed.is_empty() {
        return Ok(());
    }
    let pinged = sqlx::query_file_scalar!("queries/chat/public/ping.sql", ID, &allowed, row.seq)
        .fetch_all(&chat.pg)
        .await?;
    metrics::counter!(PINGS, "result" => "sent").increment(pinged.len() as u64);
    bump::conversation(chat, ID, row.seq, &pinged).await
}
