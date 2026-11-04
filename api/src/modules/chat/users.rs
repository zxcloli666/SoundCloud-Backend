use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::Serialize;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::members::contacts;
use crate::modules::chat::model::MemberState;
use crate::modules::chat::presence::{self, PresenceView};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, blocks};

pub const USER_ID_MAX: usize = 32;
pub const NEWCOMER_DAYS: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    DmNew,
    Media,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::All => "all",
            Scope::DmNew => "dm_new",
            Scope::Media => "media",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "all" => Some(Scope::All),
            "dm_new" => Some(Scope::DmNew),
            "media" => Some(Scope::Media),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Restriction {
    pub until: DateTime<Utc>,
    pub scope: Vec<String>,
}

impl Restriction {
    pub fn covers(&self, scope: Scope) -> bool {
        self.scope
            .iter()
            .any(|held| held == Scope::All.as_str() || held == scope.as_str())
    }

    pub fn muted(&self) -> ChatError {
        ChatError::Muted {
            until: self.until.timestamp_millis(),
        }
    }
}

pub fn permanent() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(9999, 1, 1, 0, 0, 0)
        .single()
        .expect("a fixed date")
}

pub fn valid_user_id(raw: &str) -> bool {
    let digits = raw.strip_prefix("lt:").unwrap_or(raw);
    !digits.is_empty() && raw.len() <= USER_ID_MAX && digits.bytes().all(|b| b.is_ascii_digit())
}

pub async fn touch(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/users/touch.sql", user_id)
        .execute(&chat.pg)
        .await?;
    Ok(())
}

pub async fn restriction(chat: &ChatState, user_id: &str) -> ChatResult<Option<Restriction>> {
    let row = sqlx::query_file!("queries/chat/users/restriction.sql", user_id)
        .fetch_optional(&chat.pg)
        .await?;
    Ok(row.map(|row| Restriction {
        until: row.restricted_until,
        scope: row.restriction_scope,
    }))
}

pub async fn check(chat: &ChatState, user_id: &str, scope: Scope) -> ChatResult<()> {
    match restriction(chat, user_id).await? {
        Some(held) if held.covers(scope) => Err(held.muted()),
        _ => Ok(()),
    }
}

pub async fn restrict(
    chat: &ChatState,
    user_id: &str,
    until: DateTime<Utc>,
    scope: &[Scope],
) -> ChatResult<Restriction> {
    let scope: Vec<String> = scope.iter().map(|s| s.as_str().to_owned()).collect();
    let row = sqlx::query_file!("queries/chat/users/restrict.sql", user_id, until, &scope)
        .fetch_one(&chat.pg)
        .await?;
    Ok(Restriction {
        until: row.restricted_until,
        scope: row.restriction_scope,
    })
}

pub async fn lift(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/users/lift.sql", user_id)
        .execute(&chat.pg)
        .await?;
    Ok(())
}

pub async fn first_seen(chat: &ChatState, user_id: &str) -> ChatResult<Option<DateTime<Utc>>> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/users/first_seen.sql", user_id)
            .fetch_one(&chat.pg)
            .await?,
    )
}

pub async fn newcomer(chat: &ChatState, user_id: &str) -> ChatResult<bool> {
    let seen = first_seen(chat, user_id).await?;
    Ok(seen.is_none_or(|at| at > Utc::now() - Duration::days(NEWCOMER_DAYS)))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CardView {
    pub user_id: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub permalink_url: Option<String>,
    pub verified: bool,
    pub blocked: bool,
    pub contact: bool,
    pub can_message: &'static str,
    pub presence: Option<PresenceView>,
}

pub async fn card(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
) -> ChatResult<Json<CardView>> {
    if !valid_user_id(&user) {
        return Err(ChatError::NotFound);
    }
    limits::admit(&chat, Bucket::ReadCard, &caller.user_id).await?;
    Ok(Json(card_for(&chat, &caller.user_id, &user).await?))
}

pub async fn card_for(chat: &ChatState, viewer: &str, user: &str) -> ChatResult<CardView> {
    let profile = sqlx::query_file!("queries/chat/users/profile.sql", user)
        .fetch_optional(&chat.pg)
        .await?;
    let blocked = blocks::blocked_by(chat, viewer, user).await?;
    let relation = contacts::relation(&chat.pg, viewer, user).await?;
    let landing = contacts::landing(chat, viewer, user, relation).await?;
    let (username, avatar_url, permalink_url, verified) = match profile {
        Some(row) => (
            Some(row.username),
            row.avatar_url,
            row.permalink_url,
            row.verified,
        ),
        None => (None, None, None, false),
    };
    Ok(CardView {
        user_id: user.to_owned(),
        username,
        avatar_url,
        permalink_url,
        verified,
        blocked,
        contact: relation.contact,
        can_message: match landing {
            Some(MemberState::Active) => "direct",
            Some(_) => "request",
            None => "unavailable",
        },
        presence: presence::of_contact(chat, viewer, user).await?,
    })
}

#[cfg(test)]
#[path = "tests/users_tests.rs"]
mod tests;
