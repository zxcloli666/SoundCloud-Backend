use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

use crate::common::admin::HandoffAuth;
use crate::error::{AppError, AppResult};
use crate::modules::rooms::RoomsService;
use crate::modules::rooms::store::{PAGE_MAX, Page};

#[derive(Debug, Deserialize)]
pub struct HandoffQuery {
    #[serde(default = "first_cursor")]
    cursor: String,
    #[serde(default = "default_count")]
    count: usize,
}

fn first_cursor() -> String {
    "0".to_owned()
}

fn default_count() -> usize {
    PAGE_MAX / 5
}

pub async fn handoff(
    _: HandoffAuth,
    State(rooms): State<Arc<RoomsService>>,
    Query(query): Query<HandoffQuery>,
) -> AppResult<Json<Page>> {
    Ok(Json(rooms.peek_page(&query.cursor, query.count).await?))
}

#[derive(Debug, Deserialize)]
pub struct AckBody {
    keys: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Acked {
    removed: usize,
}

pub async fn handoff_ack(
    _: HandoffAuth,
    State(rooms): State<Arc<RoomsService>>,
    Json(body): Json<AckBody>,
) -> AppResult<Json<Acked>> {
    if body.keys.len() > PAGE_MAX {
        return Err(AppError::bad_request("Too many keys in one ack"));
    }
    Ok(Json(Acked {
        removed: rooms.ack_page(&body.keys).await?,
    }))
}
