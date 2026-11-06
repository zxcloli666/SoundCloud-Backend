use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;

use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::members::roles;
use crate::modules::chat::model::MemberRole;
use crate::modules::chat::public_chat::{self, ID, log, staff, watchers};
use crate::modules::chat::sync::bump;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const MUTE_SECONDS: [u32; 3] = [600, 3_600, 86_400];
pub const REASON_CHARS_MAX: usize = 200;

#[derive(Debug, Clone, Deserialize)]
pub struct MuteBody {
    pub seconds: u32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BanBody {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Muted {
    pub until: i64,
}

fn reason_of(raw: Option<String>) -> ChatResult<Option<String>> {
    let reason = raw
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    match reason {
        Some(text) if text.chars().count() > REASON_CHARS_MAX => Err(ChatError::Invalid),
        other => Ok(other),
    }
}

async fn target_role(conn: &mut PgConnection, user: &str) -> ChatResult<MemberRole> {
    let found = sqlx::query_file!("queries/chat/members/state_of.sql", ID, user)
        .fetch_optional(&mut *conn)
        .await?;
    match found {
        Some(row) => roles::role(&row.role),
        None => Ok(MemberRole::Member),
    }
}

async fn begin(
    chat: &ChatState,
    caller: &ChatCaller,
    user: &str,
) -> ChatResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    public_chat::ensure_on(chat)?;
    limits::admit(chat, Bucket::PublicMod, &caller.user_id).await?;
    staff::valid_target(user)?;
    let actor = staff::staff_role(chat, &caller.user_id).await?;
    let mut tx = chat.pg.begin().await?;
    let target = target_role(&mut tx, user).await?;
    if !roles::may_remove(actor, target) {
        tx.rollback().await?;
        return Err(ChatError::NotMember);
    }
    Ok(tx)
}

pub async fn mute(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
    ChatJson(body): ChatJson<MuteBody>,
) -> ChatResult<Json<Muted>> {
    if !MUTE_SECONDS.contains(&body.seconds) {
        return Err(ChatError::Invalid);
    }
    let reason = reason_of(body.reason)?;
    let mut tx = begin(&chat, &caller, &user).await?;
    let until = sqlx::query_file_scalar!(
        "queries/chat/public/mute.sql",
        user,
        i32::try_from(body.seconds).map_err(|_| ChatError::Invalid)?,
        caller.user_id,
        reason
    )
    .fetch_one(&mut *tx)
    .await?
    .timestamp_millis();
    log::record_in(
        &mut tx,
        &caller.user_id,
        "mute",
        &format!("user:{user}"),
        json!({ "until": until, "seconds": body.seconds, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(Muted { until }))
}

pub async fn unmute(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
) -> ChatResult<StatusCode> {
    let mut tx = begin(&chat, &caller, &user).await?;
    let lifted = sqlx::query_file_scalar!("queries/chat/public/unmute.sql", user)
        .fetch_optional(&mut *tx)
        .await?;
    if lifted.is_none() {
        tx.rollback().await?;
        return Err(ChatError::NotFound);
    }
    log::record_in(
        &mut tx,
        &caller.user_id,
        "unmute",
        &format!("user:{user}"),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn ban(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
    ChatJson(body): ChatJson<BanBody>,
) -> ChatResult<StatusCode> {
    let reason = reason_of(body.reason)?;
    let mut tx = begin(&chat, &caller, &user).await?;
    sqlx::query_file!("queries/chat/public/ban.sql", ID, user)
        .execute(&mut *tx)
        .await?;
    log::record_in(
        &mut tx,
        &caller.user_id,
        "ban",
        &format!("user:{user}"),
        json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    watchers::remove(&chat, &user).await?;
    bump::conversation(&chat, ID, 0, std::slice::from_ref(&user)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unban(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
) -> ChatResult<StatusCode> {
    let mut tx = begin(&chat, &caller, &user).await?;
    let lifted = sqlx::query_file_scalar!("queries/chat/public/unban.sql", ID, user)
        .fetch_optional(&mut *tx)
        .await?;
    if lifted.is_none() {
        tx.rollback().await?;
        return Err(ChatError::NotFound);
    }
    log::record_in(
        &mut tx,
        &caller.user_id,
        "unban",
        &format!("user:{user}"),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    bump::conversation(&chat, ID, 0, std::slice::from_ref(&user)).await?;
    Ok(StatusCode::NO_CONTENT)
}
