use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::AUDIT_ADMIN;
use crate::modules::chat::users::{self, Restriction, Scope};
use crate::modules::chat::{ChatResult, ChatState};

pub const AUTO_WEIGHT: f32 = 3.0;
pub const AUTO_CONVERSATIONS: i32 = 2;
pub const AUTO_WINDOW_HOURS: i32 = 24;
pub const AUTO_RESTRICT_HOURS: i64 = 24;
pub const AUTO_SCOPE: [Scope; 2] = [Scope::DmNew, Scope::Media];

pub struct Pressure {
    pub weight: f32,
    pub conversations: i32,
}

impl Pressure {
    pub fn restricts(&self) -> bool {
        self.weight >= AUTO_WEIGHT && self.conversations >= AUTO_CONVERSATIONS
    }
}

pub async fn pressure(chat: &ChatState, target: &str) -> ChatResult<Pressure> {
    let row = sqlx::query_file!(
        "queries/chat/reports/weighted_against.sql",
        target,
        AUTO_WINDOW_HOURS
    )
    .fetch_one(&chat.pg)
    .await?;
    Ok(Pressure {
        weight: row.weight,
        conversations: row.conversations,
    })
}

pub async fn apply(
    chat: &ChatState,
    target: &str,
    report: Uuid,
) -> ChatResult<Option<Restriction>> {
    let pressure = pressure(chat, target).await?;
    if !pressure.restricts() {
        return Ok(None);
    }
    let until = Utc::now() + Duration::hours(AUTO_RESTRICT_HOURS);
    let restriction = users::restrict(chat, target, until, &AUTO_SCOPE).await?;
    audit::record(
        chat,
        AUDIT_ADMIN,
        "restrict",
        &audit::user(target),
        json!({
            "source": "rules",
            "report": report,
            "weight": pressure.weight,
            "conversations": pressure.conversations,
            "until": restriction.until.timestamp_millis(),
            "scope": restriction.scope,
        }),
    )
    .await?;
    Ok(Some(restriction))
}
