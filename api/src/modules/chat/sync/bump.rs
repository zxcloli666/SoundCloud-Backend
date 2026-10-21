use std::sync::LazyLock;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::sync::changes::Change;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

static BUMP: LazyLock<Script> = LazyLock::new(|| Script::new(include_str!("../lua/bump.lua")));

pub const RECIPIENTS_PER_CALL: usize = 256;
pub const GONE_SEQS_PER_ENTRY: usize = 64;

pub async fn conversation(
    chat: &ChatState,
    conversation: Uuid,
    last_seq: i64,
    recipients: &[String],
) -> ChatResult<()> {
    run(
        chat,
        Some((conversation, last_seq)),
        &[Change::Conversation(conversation)],
        recipients,
    )
    .await
}

pub async fn messages_gone(
    chat: &ChatState,
    conversation: Uuid,
    seqs: &[i64],
    recipients: &[String],
) -> ChatResult<()> {
    let changes: Vec<Change> = seqs
        .chunks(GONE_SEQS_PER_ENTRY)
        .map(|chunk| Change::GoneMessages(conversation, chunk.to_vec()))
        .collect();
    run(chat, None, &changes, recipients).await
}

pub async fn devices(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    run(chat, None, &[Change::Devices], &[user_id.to_owned()]).await
}

pub async fn requests(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    run(chat, None, &[Change::Requests], &[user_id.to_owned()]).await
}

async fn run(
    chat: &ChatState,
    seq: Option<(Uuid, i64)>,
    changes: &[Change],
    recipients: &[String],
) -> ChatResult<()> {
    let mut recipients = recipients.to_vec();
    recipients.sort();
    recipients.dedup();
    let seq = seq.filter(|(_, last_seq)| *last_seq > 0);
    if changes.is_empty() || (recipients.is_empty() && seq.is_none()) {
        return Ok(());
    }
    let members: Vec<String> = changes.iter().map(Change::member).collect();
    let mut batches: Vec<&[String]> = recipients.chunks(RECIPIENTS_PER_CALL).collect();
    if batches.is_empty() {
        batches.push(&[]);
    }
    let mut conn = chat.redis.get().await?;
    for (index, batch) in batches.into_iter().enumerate() {
        let mut call = BUMP.prepare_invoke();
        let batch_seq = seq.filter(|_| index == 0);
        if let Some((conversation, _)) = batch_seq {
            call.key(rt_keys::conversation_seq(conversation));
        }
        for recipient in batch {
            call.key(rt_keys::version(recipient))
                .key(rt_keys::deltas(recipient));
        }
        call.arg(rt_keys::VERSION_TTL.as_secs())
            .arg(rt_keys::DELTAS_KEPT)
            .arg(batch_seq.map_or(0, |(_, last_seq)| last_seq))
            .arg(&members);
        let _: i64 = call.invoke_async(&mut conn).await?;
    }
    drop(conn);
    for recipient in &recipients {
        chat.hub.wake(recipient);
    }
    Ok(())
}
