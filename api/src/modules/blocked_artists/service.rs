use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::sc_ids::{EntityRef, user_id_variants};
use crate::error::{AppError, AppResult};

pub const MAX_BLOCKED: i64 = 500;
const MAX_LINKED_ACCOUNTS: usize = 16;
const MAX_NAME_LEN: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockKind {
    User,
    Artist,
}

impl BlockKind {
    pub fn parse(raw: &str) -> AppResult<Self> {
        match raw {
            "user" => Ok(Self::User),
            "artist" => Ok(Self::Artist),
            _ => Err(AppError::bad_request("invalid block kind")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Artist => "artist",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockedArtist {
    pub kind: BlockKind,
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub sc_user_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BlockInput {
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub sc_user_ids: Vec<String>,
}

pub fn normalize_target(kind: BlockKind, raw: &str) -> AppResult<String> {
    match kind {
        BlockKind::User => EntityRef::user(raw)
            .map(EntityRef::sc_id)
            .ok_or_else(|| AppError::bad_request("invalid user identifier")),
        BlockKind::Artist => Uuid::parse_str(raw.trim())
            .map(|u| u.to_string())
            .map_err(|_| AppError::bad_request("invalid artist identifier")),
    }
}

pub fn linked_accounts(kind: BlockKind, target: &str, raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if kind == BlockKind::User {
        out.push(target.to_string());
    }
    for id in raw
        .iter()
        .filter_map(|r| EntityRef::user(r))
        .map(EntityRef::sc_id)
    {
        if out.len() >= MAX_LINKED_ACCOUNTS {
            break;
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

fn clean_name(raw: &str) -> AppResult<String> {
    let name: String = raw.trim().chars().take(MAX_NAME_LEN).collect();
    if name.is_empty() {
        return Err(AppError::bad_request("name is required"));
    }
    Ok(name)
}

fn row(
    kind: String,
    id: String,
    name: String,
    avatar_url: Option<String>,
    sc_user_ids: Vec<String>,
    created_at: DateTime<Utc>,
) -> Option<BlockedArtist> {
    Some(BlockedArtist {
        kind: BlockKind::parse(&kind).ok()?,
        id,
        name,
        avatar_url,
        sc_user_ids,
        created_at,
    })
}

pub async fn list(pg: &PgPool, sc_user_id: &str) -> AppResult<Vec<BlockedArtist>> {
    let variants = user_id_variants(sc_user_id);
    let mut out: Vec<BlockedArtist> =
        sqlx::query_file!("queries/blocked_artists/list.sql", &variants)
            .fetch_all(pg)
            .await?
            .into_iter()
            .filter_map(|r| {
                row(
                    r.kind,
                    r.target_id,
                    r.name,
                    r.avatar_url,
                    r.sc_user_ids,
                    r.created_at,
                )
            })
            .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Ok(out)
}

pub async fn block(
    pg: &PgPool,
    sc_user_id: &str,
    kind: BlockKind,
    raw_id: &str,
    input: &BlockInput,
) -> AppResult<BlockedArtist> {
    let target = normalize_target(kind, raw_id)?;
    let name = clean_name(&input.name)?;
    let accounts = linked_accounts(kind, &target, &input.sc_user_ids);
    let avatar = input
        .avatar_url
        .as_deref()
        .map(str::trim)
        .filter(|s| s.starts_with("https://"))
        .map(str::to_string);
    let variants = user_id_variants(sc_user_id);
    let others = sqlx::query_file_scalar!(
        "queries/blocked_artists/count.sql",
        &variants,
        kind.as_str(),
        &target
    )
    .fetch_one(pg)
    .await?;
    if others >= MAX_BLOCKED {
        return Err(AppError::bad_request("blocklist is full"));
    }
    let r = sqlx::query_file!(
        "queries/blocked_artists/upsert.sql",
        sc_user_id,
        kind.as_str(),
        &target,
        &name,
        avatar.as_deref(),
        &accounts
    )
    .fetch_one(pg)
    .await?;
    row(
        r.kind,
        r.target_id,
        r.name,
        r.avatar_url,
        r.sc_user_ids,
        r.created_at,
    )
    .ok_or_else(|| AppError::bad_request("invalid block kind"))
}

pub async fn unblock(
    pg: &PgPool,
    sc_user_id: &str,
    kind: BlockKind,
    raw_id: &str,
) -> AppResult<()> {
    let target = normalize_target(kind, raw_id)?;
    let variants = user_id_variants(sc_user_id);
    sqlx::query_file!(
        "queries/blocked_artists/remove.sql",
        &variants,
        kind.as_str(),
        &target
    )
    .execute(pg)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
