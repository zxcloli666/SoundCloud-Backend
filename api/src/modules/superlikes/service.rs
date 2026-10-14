use axum::http::StatusCode;
use serde::Serialize;
use sqlx::{PgConnection, PgPool};

use crate::common::sc_ids::{extract_sc_id, user_id_variants};
use crate::error::{AppError, AppResult};

pub const FREE_LIMIT: i64 = 10;

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

pub async fn like_is_alive(pg: &PgPool, sc_user_id: &str, sc_track_id: &str) -> AppResult<bool> {
    let variants = user_id_variants(sc_user_id);
    let alive = sqlx::query_file_scalar!(
        "queries/superlikes/like_is_alive.sql",
        &variants,
        sc_track_id
    )
    .fetch_one(pg)
    .await?;
    Ok(alive)
}

pub async fn ensure_room(pg: &PgPool, sc_user_id: &str, premium: bool) -> AppResult<()> {
    if quota(pg, sc_user_id, premium).await?.is_full() {
        return Err(limit_reached());
    }
    Ok(())
}

pub async fn superlike(
    pg: &PgPool,
    sc_user_id: &str,
    sc_track_id: &str,
    premium: bool,
) -> AppResult<Quota> {
    let variants = user_id_variants(sc_user_id);
    let mut transaction = pg.begin().await?;
    sqlx::query_file_scalar!(
        "queries/superlikes/lock_user.sql",
        extract_sc_id(sc_user_id)
    )
    .fetch_one(&mut *transaction)
    .await?;
    let used = live_count(&mut transaction, &variants).await?;
    if is_live(&mut transaction, &variants, sc_track_id).await? {
        return Ok(Quota::of(used, premium));
    }
    if Quota::of(used, premium).is_full() {
        return Err(limit_reached());
    }
    let written = sqlx::query_file!("queries/superlikes/insert.sql", &variants, sc_track_id)
        .execute(&mut *transaction)
        .await?;
    if written.rows_affected() == 0 {
        return Err(AppError::conflict("only a liked track can be superliked"));
    }
    transaction.commit().await?;
    Ok(Quota::of(used + 1, premium))
}

pub async fn forget(pg: &PgPool, sc_user_id: &str, sc_track_id: &str) -> AppResult<()> {
    let variants = user_id_variants(sc_user_id);
    sqlx::query_file!("queries/superlikes/remove.sql", &variants, sc_track_id)
        .execute(pg)
        .await?;
    Ok(())
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

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
