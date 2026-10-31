use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{PgConnection, PgPool};

use crate::common::sc_ids::{EntityKind, extract_sc_id, require_ref, user_id_variants};
use crate::error::{AppError, AppResult};

const STEP: i64 = 1 << 32;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum LikesOrderRequest {
    MoveAfter { track: String, after: String },
    MoveToTop { track: String },
    MoveToBottom { track: String },
    Reset,
}

enum Target<Anchor> {
    Top,
    Bottom,
    After(Anchor),
}

struct Owner {
    id: String,
    variants: Vec<String>,
}

struct Bounds {
    low: Option<i64>,
    high: Option<i64>,
}

struct Like {
    sc_track_id: String,
    created_at: DateTime<Utc>,
}

pub async fn apply(pg: &PgPool, sc_user_id: &str, request: LikesOrderRequest) -> AppResult<()> {
    let owner = Owner {
        id: extract_sc_id(sc_user_id).to_owned(),
        variants: user_id_variants(sc_user_id),
    };
    let mut tx = crate::db::begin_long(pg).await?;
    sqlx::query_file_scalar!("queries/likes/order/lock.sql", &owner.id)
        .fetch_one(&mut *tx)
        .await?;
    match request {
        LikesOrderRequest::Reset => {
            sqlx::query_file!("queries/likes/order/reset.sql", &owner.id)
                .execute(&mut *tx)
                .await?;
        }
        LikesOrderRequest::MoveToTop { track } => {
            move_track(&mut tx, &owner, &track_id(&track)?, Target::Top).await?;
        }
        LikesOrderRequest::MoveToBottom { track } => {
            move_track(&mut tx, &owner, &track_id(&track)?, Target::Bottom).await?;
        }
        LikesOrderRequest::MoveAfter { track, after } => {
            let target = Target::After(track_id(&after)?);
            move_track(&mut tx, &owner, &track_id(&track)?, target).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub async fn forget(pg: &PgPool, sc_user_id: &str, sc_track_id: &str) -> AppResult<()> {
    sqlx::query_file!(
        "queries/likes/order/forget.sql",
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
    owner: &Owner,
    sc_track_id: &str,
    target: Target<String>,
) -> AppResult<()> {
    let anchor_id = match &target {
        Target::After(anchor) if anchor == sc_track_id => {
            return Err(AppError::bad_request("a track cannot follow itself"));
        }
        Target::After(anchor) => Some(anchor.as_str()),
        Target::Top | Target::Bottom => None,
    };
    let likes = live_likes(tx, owner, [Some(sc_track_id), anchor_id]).await?;
    let like_of = |id: &str| {
        likes
            .iter()
            .find(|like| like.sc_track_id == id)
            .ok_or_else(|| {
                AppError::coded(
                    StatusCode::NOT_FOUND,
                    "like_not_found",
                    "track is not among your likes",
                )
            })
    };
    let track = like_of(sc_track_id)?;
    let target = match target {
        Target::Top => Target::Top,
        Target::Bottom => Target::Bottom,
        Target::After(anchor) => Target::After(like_of(&anchor)?),
    };
    rank_unranked(tx, owner, false).await?;
    let rank = match free_rank(tx, owner, sc_track_id, &target).await? {
        Some(rank) => rank,
        None => {
            renumber(tx, owner).await?;
            free_rank(tx, owner, sc_track_id, &target)
                .await?
                .ok_or_else(|| AppError::internal("no room left in the like order"))?
        }
    };
    sqlx::query_file!(
        "queries/likes/order/place.sql",
        &owner.id,
        sc_track_id,
        track.created_at,
        rank
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn live_likes(
    tx: &mut PgConnection,
    owner: &Owner,
    ids: [Option<&str>; 2],
) -> AppResult<Vec<Like>> {
    let ids: Vec<String> = ids.into_iter().flatten().map(str::to_owned).collect();
    Ok(sqlx::query_file_as!(
        Like,
        "queries/likes/order/likes.sql",
        &owner.variants,
        &owner.id,
        &ids
    )
    .fetch_all(&mut *tx)
    .await?)
}

async fn rank_unranked(tx: &mut PgConnection, owner: &Owner, everything: bool) -> AppResult<()> {
    sqlx::query_file!(
        "queries/likes/order/rank_unranked.sql",
        &owner.variants,
        &owner.id,
        STEP,
        everything
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn renumber(tx: &mut PgConnection, owner: &Owner) -> AppResult<()> {
    sqlx::query_file!(
        "queries/likes/order/drop_stale.sql",
        &owner.variants,
        &owner.id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query_file!("queries/likes/order/renumber.sql", &owner.id, STEP)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn free_rank(
    tx: &mut PgConnection,
    owner: &Owner,
    sc_track_id: &str,
    target: &Target<&Like>,
) -> AppResult<Option<i64>> {
    let anchor = match target {
        Target::Top => {
            let low = bounds(tx, owner, sc_track_id).await?.low;
            return Ok(low.map_or(Some(0), |low| low.checked_sub(STEP)));
        }
        Target::Bottom => {
            let high = bounds(tx, owner, sc_track_id).await?.high;
            return Ok(high.map_or(Some(0), |high| high.checked_add(STEP)));
        }
        Target::After(anchor) => anchor,
    };
    let anchor_rank = anchor_rank(tx, owner, anchor).await?;
    let next = sqlx::query_file_scalar!(
        "queries/likes/order/next_rank.sql",
        &owner.id,
        anchor_rank,
        sc_track_id
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(match next {
        None => anchor_rank.checked_add(STEP),
        Some(next) => next
            .checked_sub(anchor_rank)
            .filter(|gap| *gap >= 2)
            .map(|gap| anchor_rank + gap / 2),
    })
}

async fn anchor_rank(tx: &mut PgConnection, owner: &Owner, anchor: &Like) -> AppResult<i64> {
    if let Some(rank) = ranked(tx, owner, anchor).await? {
        return Ok(rank);
    }
    rank_unranked(tx, owner, true).await?;
    ranked(tx, owner, anchor)
        .await?
        .ok_or_else(|| AppError::internal("the anchor like has no rank"))
}

async fn ranked(tx: &mut PgConnection, owner: &Owner, like: &Like) -> AppResult<Option<i64>> {
    Ok(sqlx::query_file_scalar!(
        "queries/likes/order/anchor_rank.sql",
        &owner.id,
        &like.sc_track_id,
        like.created_at
    )
    .fetch_optional(&mut *tx)
    .await?)
}

async fn bounds(tx: &mut PgConnection, owner: &Owner, sc_track_id: &str) -> AppResult<Bounds> {
    Ok(sqlx::query_file_as!(
        Bounds,
        "queries/likes/order/bounds.sql",
        &owner.id,
        sc_track_id
    )
    .fetch_one(&mut *tx)
    .await?)
}
