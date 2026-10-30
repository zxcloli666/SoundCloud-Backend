use axum::http::StatusCode;
use serde::Serialize;
use sqlx::{PgConnection, PgPool};

use crate::common::sc_ids::{EntityRef, extract_sc_id, user_id_variants};
use crate::error::{AppError, AppResult};
use crate::modules::likes::service::like_track_on;
use crate::modules::superlikes::order;
use crate::modules::sync_queue::SyncQueueService;

pub const FREE_LIMIT: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Superliked {
    pub quota: Quota,
    pub newly_liked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Quota {
    pub used: i64,
    pub limit: Option<i64>,
}

impl Quota {
    fn of(used: i64, premium: bool) -> Self {
        Self {
            used,
            limit: (!premium).then_some(FREE_LIMIT),
        }
    }

    fn is_full(self) -> bool {
        self.limit.is_some_and(|limit| self.used >= limit)
    }
}

pub fn limit_reached() -> AppError {
    AppError::coded(
        StatusCode::PAYMENT_REQUIRED,
        "superlike_limit_reached",
        format!("Without Star up to {FREE_LIMIT} superlikes can be active at a time"),
    )
}

pub async fn quota(pg: &PgPool, sc_user_id: &str, premium: bool) -> AppResult<Quota> {
    let mut connection = pg.acquire().await?;
    let used = live_count(&mut connection, &user_id_variants(sc_user_id)).await?;
    Ok(Quota::of(used, premium))
}

pub async fn superlike(
    pg: &PgPool,
    sync_queue: &SyncQueueService,
    sc_user_id: &str,
    track: &EntityRef,
    premium: bool,
) -> AppResult<Superliked> {
    crate::db::retry_conflicts(|| superlike_once(pg, sync_queue, sc_user_id, track, premium)).await
}

async fn superlike_once(
    pg: &PgPool,
    sync_queue: &SyncQueueService,
    sc_user_id: &str,
    track: &EntityRef,
    premium: bool,
) -> AppResult<Superliked> {
    let variants = user_id_variants(sc_user_id);
    let sc_track_id = &track.sc_id();
    let mut transaction = pg.begin().await?;
    sqlx::query_file_scalar!(
        "queries/superlikes/lock_user.sql",
        extract_sc_id(sc_user_id)
    )
    .fetch_one(&mut *transaction)
    .await?;
    let used = live_count(&mut transaction, &variants).await?;
    if is_live(&mut transaction, &variants, sc_track_id).await? {
        return Ok(Superliked {
            quota: Quota::of(used, premium),
            newly_liked: false,
        });
    }
    if Quota::of(used, premium).is_full() {
        return Err(limit_reached());
    }
    let newly_liked = if is_liked(&mut transaction, &variants, sc_track_id).await? {
        false
    } else {
        like_track_on(&mut transaction, sync_queue, sc_user_id, track).await?
    };
    let written = sqlx::query_file!("queries/superlikes/insert.sql", &variants, sc_track_id)
        .execute(&mut *transaction)
        .await?;
    if written.rows_affected() == 0 {
        return Err(AppError::conflict("only a liked track can be superliked"));
    }
    transaction.commit().await?;
    Ok(Superliked {
        quota: Quota::of(used + 1, premium),
        newly_liked,
    })
}

pub async fn forget(pg: &PgPool, sc_user_id: &str, sc_track_id: &str) -> AppResult<()> {
    let variants = user_id_variants(sc_user_id);
    sqlx::query_file!("queries/superlikes/remove.sql", &variants, sc_track_id)
        .execute(pg)
        .await?;
    order::forget(pg, sc_user_id, sc_track_id).await
}

async fn live_count(connection: &mut PgConnection, variants: &[String]) -> AppResult<i64> {
    let used = sqlx::query_file_scalar!("queries/superlikes/live_count.sql", variants)
        .fetch_one(connection)
        .await?;
    Ok(used)
}

async fn is_live(
    connection: &mut PgConnection,
    variants: &[String],
    sc_track_id: &str,
) -> AppResult<bool> {
    let live = sqlx::query_file_scalar!("queries/superlikes/is_live.sql", variants, sc_track_id)
        .fetch_one(connection)
        .await?;
    Ok(live)
}

async fn is_liked(
    connection: &mut PgConnection,
    variants: &[String],
    sc_track_id: &str,
) -> AppResult<bool> {
    let liked = sqlx::query_file_scalar!("queries/superlikes/is_liked.sql", variants, sc_track_id)
        .fetch_one(connection)
        .await?;
    Ok(liked)
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
