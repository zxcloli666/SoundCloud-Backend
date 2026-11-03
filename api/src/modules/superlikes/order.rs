use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};

use crate::common::sc_ids::{EntityKind, extract_sc_id, require_ref, user_id_variants};
use crate::error::{AppError, AppResult};
use crate::modules::likes::LikesOrderRequest;

const STEP: i64 = 1 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Superlike {
    pub sc_track_id: String,
    pub created_at: DateTime<Utc>,
    pub rank: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Placed {
    pub sc_track_id: String,
    pub created_at: DateTime<Utc>,
    pub rank: i64,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Target<'a> {
    Top,
    Bottom,
    After(&'a str),
}

pub async fn apply(pg: &PgPool, sc_user_id: &str, request: LikesOrderRequest) -> AppResult<()> {
    let owner = extract_sc_id(sc_user_id);
    let variants = user_id_variants(sc_user_id);
    let mut tx = crate::db::begin_long(pg).await?;
    sqlx::query_file_scalar!("queries/superlikes/lock_user.sql", owner)
        .fetch_one(&mut *tx)
        .await?;
    match request {
        LikesOrderRequest::Reset => {
            sqlx::query_file!("queries/superlikes/order/reset.sql", owner)
                .execute(&mut *tx)
                .await?;
        }
        LikesOrderRequest::MoveToTop { track } => {
            move_track(&mut tx, owner, &variants, &track_id(&track)?, Target::Top).await?;
        }
        LikesOrderRequest::MoveToBottom { track } => {
            move_track(
                &mut tx,
                owner,
                &variants,
                &track_id(&track)?,
                Target::Bottom,
            )
            .await?;
        }
        LikesOrderRequest::MoveAfter { track, after } => {
            let after = track_id(&after)?;
            let target = Target::After(&after);
            move_track(&mut tx, owner, &variants, &track_id(&track)?, target).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub async fn forget(pg: &PgPool, sc_user_id: &str, sc_track_id: &str) -> AppResult<()> {
    sqlx::query_file!(
        "queries/superlikes/order/forget.sql",
        extract_sc_id(sc_user_id),
        sc_track_id
    )
    .execute(pg)
    .await?;
    Ok(())
}

fn track_id(input: &str) -> AppResult<String> {
    Ok(require_ref(EntityKind::Track, input)?.sc_id())
}

async fn move_track(
    tx: &mut PgConnection,
    owner: &str,
    variants: &[String],
    sc_track_id: &str,
    target: Target<'_>,
) -> AppResult<()> {
    sqlx::query_file!("queries/superlikes/order/drop_stale.sql", variants, owner)
        .execute(&mut *tx)
        .await?;
    let listed = sqlx::query_file_as!(
        Superlike,
        "queries/superlikes/order/listed.sql",
        variants,
        owner
    )
    .fetch_all(&mut *tx)
    .await?;
    let changed = arrange(&listed, sc_track_id, target)?;
    if changed.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = changed.iter().map(|p| p.sc_track_id.clone()).collect();
    let dates: Vec<DateTime<Utc>> = changed.iter().map(|p| p.created_at).collect();
    let ranks: Vec<i64> = changed.iter().map(|p| p.rank).collect();
    sqlx::query_file!(
        "queries/superlikes/order/place.sql",
        owner,
        &ids,
        &dates,
        &ranks
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(super) fn arrange(
    listed: &[Superlike],
    sc_track_id: &str,
    target: Target<'_>,
) -> AppResult<Vec<Placed>> {
    if matches!(target, Target::After(anchor) if anchor == sc_track_id) {
        return Err(AppError::bad_request("a track cannot follow itself"));
    }
    let mut order = ranked(listed);
    let from = position(&order, sc_track_id)?;
    let moving = order.remove(from);
    let at = match target {
        Target::Top => 0,
        Target::Bottom => order.len(),
        Target::After(anchor) => position(&order, anchor)? + 1,
    };
    let before = at.checked_sub(1).map(|i| order[i].rank);
    let after = order.get(at).map(|placed| placed.rank);
    order.insert(at, moving);
    match between(before, after) {
        Some(rank) => order[at].rank = rank,
        None => renumber(&mut order),
    }
    Ok(order
        .into_iter()
        .filter(|placed| !kept(listed, placed))
        .collect())
}

fn position(order: &[Placed], sc_track_id: &str) -> AppResult<usize> {
    order
        .iter()
        .position(|placed| placed.sc_track_id == sc_track_id)
        .ok_or_else(|| {
            AppError::coded(
                StatusCode::NOT_FOUND,
                "superlike_not_found",
                "track is not among your superlikes",
            )
        })
}

fn ranked(listed: &[Superlike]) -> Vec<Placed> {
    let unranked = listed.iter().take_while(|s| s.rank.is_none()).count();
    let top = listed.get(unranked).and_then(|s| s.rank).unwrap_or(0);
    let ranks: Option<Vec<i64>> = listed
        .iter()
        .enumerate()
        .map(|(i, s)| s.rank.or_else(|| above(top, unranked.checked_sub(i)?)))
        .collect();
    let ranks = ranks.unwrap_or_else(|| spaced(listed.len()));
    listed
        .iter()
        .zip(ranks)
        .map(|(s, rank)| Placed {
            sc_track_id: s.sc_track_id.clone(),
            created_at: s.created_at,
            rank,
        })
        .collect()
}

fn above(top: i64, steps: usize) -> Option<i64> {
    STEP.checked_mul(steps as i64)
        .and_then(|gap| top.checked_sub(gap))
}

fn between(before: Option<i64>, after: Option<i64>) -> Option<i64> {
    match (before, after) {
        (None, None) => Some(0),
        (None, Some(after)) => after.checked_sub(STEP),
        (Some(before), None) => before.checked_add(STEP),
        (Some(before), Some(after)) => after
            .checked_sub(before)
            .filter(|gap| *gap >= 2)
            .map(|gap| before + gap / 2),
    }
}

fn renumber(order: &mut [Placed]) {
    let ranks = spaced(order.len());
    for (placed, rank) in order.iter_mut().zip(ranks) {
        placed.rank = rank;
    }
}

fn spaced(count: usize) -> Vec<i64> {
    (1..=count as i64).map(|i| i * STEP).collect()
}

fn kept(listed: &[Superlike], placed: &Placed) -> bool {
    listed
        .iter()
        .any(|s| s.sc_track_id == placed.sc_track_id && s.rank == Some(placed.rank))
}

#[cfg(test)]
#[path = "order_tests.rs"]
mod tests;
