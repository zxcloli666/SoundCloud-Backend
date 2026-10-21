use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::{ChatResult, ChatState};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Stats {
    pub users: i64,
    pub devices: i64,
    pub direct: i64,
    pub groups: i64,
    pub rooms: i64,
    pub reports_open: i64,
    pub reports_unjudged: i64,
    pub reports_day: i64,
    pub actioned_day: i64,
    pub rejected_day: i64,
    pub restricted: i64,
    pub ai_enabled: bool,
}

pub async fn stats(_: ChatAdmin, State(chat): State<Arc<ChatState>>) -> ChatResult<Json<Stats>> {
    let row = sqlx::query_file!("queries/chat/admin/stats.sql")
        .fetch_one(&chat.pg)
        .await?;
    Ok(Json(Stats {
        users: row.users,
        devices: row.devices,
        direct: row.direct,
        groups: row.groups,
        rooms: row.rooms,
        reports_open: row.reports_open,
        reports_unjudged: row.reports_unjudged,
        reports_day: row.reports_day,
        actioned_day: row.actioned_day,
        rejected_day: row.rejected_day,
        restricted: row.restricted,
        ai_enabled: chat.moderator.ai_enabled(),
    }))
}

pub async fn metrics(_: ChatAdmin, State(chat): State<Arc<ChatState>>) -> Response {
    match crate::metrics::render(&chat.pg).await {
        Some(body) => (
            [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
            body,
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder is not installed",
        )
            .into_response(),
    }
}
