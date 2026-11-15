use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::common::admin::AdminAuth;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const DEFAULT_LIMIT: i64 = 500;
const MAX_LIMIT: i64 = 2000;
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
pub struct ActivityQuery {
    pub since: i64,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct UserActivity {
    pub user_id: String,
    pub active_days_28: i64,
    pub plays_28: i64,
    pub first_play_at: i64,
    pub last_play_at: i64,
}

#[derive(Serialize, Debug)]
pub struct ActivityPage {
    pub users: Vec<UserActivity>,
    pub next: Option<String>,
}

#[tracing::instrument(skip_all)]
pub async fn activity(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ActivityQuery>,
) -> AppResult<Json<ActivityPage>> {
    let since = DateTime::from_timestamp(q.since, 0)
        .ok_or_else(|| AppError::bad_request("since must be unix seconds"))?
        .naive_utc();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let after = q.after.unwrap_or_default();
    Ok(Json(page(&state.pg, since, &after, limit).await?))
}

pub async fn page(
    pool: &PgPool,
    since: NaiveDateTime,
    after: &str,
    limit: i64,
) -> AppResult<ActivityPage> {
    let mut tx = crate::db::begin_with_timeout(pool, STATEMENT_TIMEOUT).await?;
    let rows = sqlx::query_file!("queries/admin/activity/page.sql", after, since, limit)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;

    let next = if rows.len() as i64 == limit {
        rows.last().map(|row| row.user_id.clone())
    } else {
        None
    };
    let mut users: Vec<UserActivity> = Vec::new();
    for row in rows {
        let (Some(plays), Some(days), Some(first)) =
            (row.plays_28, row.active_days_28, row.first_play_at)
        else {
            continue;
        };
        let user_id = bare_id(&row.user_id);
        if users.iter().any(|u| u.user_id == user_id) {
            continue;
        }
        users.push(UserActivity {
            user_id,
            active_days_28: days,
            plays_28: plays,
            first_play_at: first.and_utc().timestamp(),
            last_play_at: row.last_play_at.and_utc().timestamp(),
        });
    }
    Ok(ActivityPage { users, next })
}

fn bare_id(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).to_owned()
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod activity_tests;
