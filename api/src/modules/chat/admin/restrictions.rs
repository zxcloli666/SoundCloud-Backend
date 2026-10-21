use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::body::ChatJson;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const SHOWN: i64 = 500;
pub const HOURS_MAX: i64 = 24 * 3_650;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RestrictionView {
    pub user_id: String,
    pub until: i64,
    pub scope: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restrict {
    pub user_id: String,
    #[serde(default)]
    pub scope: Vec<String>,
    pub hours: Option<i64>,
    #[serde(default)]
    pub permanent: bool,
    #[serde(default)]
    pub lift: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RestrictAnswer {
    pub user_id: String,
    pub until: Option<i64>,
    pub scope: Vec<String>,
}

pub async fn list(
    _: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Vec<RestrictionView>>> {
    let rows = sqlx::query_file!("queries/chat/admin/restrictions_list.sql", SHOWN)
        .fetch_all(&chat.pg)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| RestrictionView {
                user_id: row.user_id,
                until: row.restricted_until.timestamp_millis(),
                scope: row.restriction_scope,
                updated_at: row.updated_at.timestamp_millis(),
            })
            .collect(),
    ))
}

pub async fn restrict(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatJson(input): ChatJson<Restrict>,
) -> ChatResult<Json<RestrictAnswer>> {
    if !users::valid_user_id(&input.user_id) {
        return Err(ChatError::Invalid);
    }
    let target = audit::user(&input.user_id);
    if input.lift {
        users::lift(&chat, &input.user_id).await?;
        audit::record(&chat, &admin.admin, "lift", &target, json!({})).await?;
        return Ok(Json(RestrictAnswer {
            user_id: input.user_id,
            until: None,
            scope: Vec::new(),
        }));
    }
    let scope = parse_scope(&input.scope)?;
    let until = match (input.permanent, input.hours) {
        (true, None) => users::permanent(),
        (false, Some(hours)) if (1..=HOURS_MAX).contains(&hours) => {
            Utc::now() + Duration::hours(hours)
        }
        _ => return Err(ChatError::Invalid),
    };
    let restriction = users::restrict(&chat, &input.user_id, until, &scope).await?;
    audit::record(
        &chat,
        &admin.admin,
        "restrict",
        &target,
        json!({
            "until": restriction.until.timestamp_millis(),
            "scope": restriction.scope,
            "permanent": input.permanent,
        }),
    )
    .await?;
    Ok(Json(RestrictAnswer {
        user_id: input.user_id,
        until: Some(restriction.until.timestamp_millis()),
        scope: restriction.scope,
    }))
}

fn parse_scope(raw: &[String]) -> ChatResult<Vec<Scope>> {
    if raw.is_empty() {
        return Ok(vec![Scope::All]);
    }
    raw.iter()
        .map(|scope| Scope::parse(scope).ok_or(ChatError::Invalid))
        .collect()
}
