use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use redis::AsyncCommands;
use tokio::time::Instant;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::sync::shards::Shards;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, rt_keys};

pub const TYPING_REPEAT: Duration = Duration::from_secs(2);

pub struct TypingBoard {
    conversations: Shards<Uuid, Vec<(String, Instant)>>,
}

impl TypingBoard {
    pub fn new() -> Self {
        Self {
            conversations: Shards::new(),
        }
    }

    pub fn repeated(&self, conversation: Uuid, user_id: &str, now: Instant) -> bool {
        self.conversations
            .lock(&conversation)
            .get(&conversation)
            .and_then(|typers| typers.iter().find(|(typer, _)| typer == user_id))
            .is_some_and(|(_, since)| now.saturating_duration_since(*since) < TYPING_REPEAT)
    }

    pub fn note(&self, conversation: Uuid, user_id: &str, now: Instant) {
        let mut conversations = self.conversations.lock(&conversation);
        let typers = conversations.entry(conversation).or_default();
        typers.retain(|(typer, since)| typer != user_id && fresh(*since, now));
        typers.push((user_id.to_owned(), now));
    }

    pub fn candidates(&self, conversation: Uuid, now: Instant) -> Vec<String> {
        self.conversations
            .lock(&conversation)
            .get(&conversation)
            .map(|typers| {
                typers
                    .iter()
                    .filter(|(_, since)| fresh(*since, now))
                    .map(|(typer, _)| typer.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn prune(&self, now: Instant, round: usize) {
        let mut conversations = self.conversations.lock_at(round);
        conversations.retain(|_, typers| {
            typers.retain(|(_, since)| fresh(*since, now));
            !typers.is_empty()
        });
    }
}

impl Default for TypingBoard {
    fn default() -> Self {
        Self::new()
    }
}

fn fresh(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) < rt_keys::TYPING_TTL
}

pub async fn typing(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    let now = Instant::now();
    if chat.hub.typing.repeated(conversation, &caller.user_id, now) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let shares = sqlx::query_file_scalar!(
        "queries/chat/sync/typing_member.sql",
        conversation,
        caller.user_id
    )
    .fetch_optional(&chat.pg)
    .await?
    .ok_or(ChatError::NotMember)?;
    if !shares {
        return Ok(StatusCode::NO_CONTENT);
    }
    {
        let mut conn = chat.redis.get().await?;
        let _: () = conn
            .set_ex(
                rt_keys::typing(conversation, &caller.user_id),
                1,
                rt_keys::TYPING_TTL.as_secs(),
            )
            .await?;
    }
    chat.hub.typing.note(conversation, &caller.user_id, now);
    chat.hub.wake_focus(conversation, &caller.user_id);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn typing_in(chat: &ChatState, conversation: Uuid) -> ChatResult<Vec<String>> {
    let candidates = chat.hub.typing.candidates(conversation, Instant::now());
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = candidates
        .iter()
        .map(|user| rt_keys::typing(conversation, user))
        .collect();
    let present: Vec<Option<String>> = {
        let mut conn = chat.redis.get().await?;
        redis::cmd("MGET").arg(&keys).query_async(&mut conn).await?
    };
    Ok(candidates
        .into_iter()
        .zip(present)
        .filter_map(|(user, present)| present.map(|_| user))
        .collect())
}
