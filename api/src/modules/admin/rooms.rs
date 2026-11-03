use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::common::admin::AdminAuth;
use crate::error::AppResult;
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
    _: AdminAuth,
    State(rooms): State<Arc<RoomsService>>,
    Query(query): Query<HandoffQuery>,
) -> AppResult<Json<Page>> {
    Ok(Json(rooms.take_page(&query.cursor, query.count).await?))
}
