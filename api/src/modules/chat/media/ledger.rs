use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::media::limits::{ATTACH_WINDOW, QUOTA_BYTES, RESTORE_WINDOW};
use crate::modules::chat::media::wire::Picked;
use crate::modules::chat::quota::{Used, seconds, signed, unsigned, used_in};
use crate::modules::chat::stores::Object;
use crate::modules::chat::{ChatError, ChatResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Released {
    pub roots: u32,
    pub freed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Restored {
    pub roots: u32,
    pub used: Used,
}

pub async fn lock(tx: &mut PgConnection, ids: &[Uuid]) -> ChatResult<Vec<Uuid>> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/media/lock_roots.sql", ids)
            .fetch_all(&mut *tx)
            .await?,
    )
}

pub async fn lock_owners(tx: &mut PgConnection, roots: &[Uuid]) -> ChatResult<()> {
    sqlx::query_file_scalar!("queries/chat/media/lock_owners.sql", roots)
        .fetch_all(&mut *tx)
        .await?;
    Ok(())
}

pub async fn settle(tx: &mut PgConnection, ids: &[Uuid]) -> ChatResult<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    lock(tx, ids).await?;
    lock_owners(tx, ids).await?;
    let settled = sqlx::query_file_scalar!("queries/chat/media/settle.sql", ids)
        .fetch_one(&mut *tx)
        .await?;
    Ok(u64::try_from(settled).unwrap_or(0))
}

pub async fn commit(
    pg: &PgPool,
    user_id: &str,
    root: Uuid,
    kept: &[Object],
) -> ChatResult<Option<Used>> {
    let key_of = |is_root: bool| {
        kept.iter()
            .find(|object| (object.id == root) == is_root)
            .map(|object| object.key.as_str())
    };
    let root_key = key_of(true).ok_or(ChatError::Internal)?;
    let mut tx = pg.begin().await?;
    lock(&mut tx, &[root]).await?;
    let row = sqlx::query_file!(
        "queries/chat/media/commit_flip.sql",
        root,
        user_id,
        seconds(ATTACH_WINDOW),
        root_key,
        key_of(false)
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if row.files == 0 {
        return Ok(None);
    }
    match (row.bytes, row.reserved) {
        (Some(bytes), Some(reserved)) => Ok(Some(Used::of(bytes, reserved))),
        _ => Ok(Some(used_in(pg, user_id).await?)),
    }
}

pub async fn release(pg: &PgPool, user_id: &str, picked: &Picked) -> ChatResult<(Released, Used)> {
    let (ids, all) = picked.parts();
    let mut tx = pg.begin().await?;
    let roots = sqlx::query_file_scalar!("queries/chat/media/delete_lock.sql", user_id, ids, all)
        .fetch_all(&mut *tx)
        .await?;
    if roots.is_empty() {
        tx.rollback().await?;
        let nothing = Released { roots: 0, freed: 0 };
        return Ok((nothing, used_in(pg, user_id).await?));
    }
    let row = sqlx::query_file!(
        "queries/chat/media/delete_rows.sql",
        user_id,
        &roots,
        seconds(RESTORE_WINDOW)
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let released = Released {
        roots: u32::try_from(row.deleted).unwrap_or(0),
        freed: unsigned(row.freed),
    };
    let used = match (row.bytes, row.reserved) {
        (Some(bytes), Some(reserved)) => Used::of(bytes, reserved),
        _ => used_in(pg, user_id).await?,
    };
    Ok((released, used))
}

pub async fn restore(pg: &PgPool, user_id: &str, picked: &Picked) -> ChatResult<Restored> {
    let (ids, all) = picked.parts();
    let mut tx = pg.begin().await?;
    let roots = sqlx::query_file_scalar!("queries/chat/media/restore_lock.sql", user_id, ids, all)
        .fetch_all(&mut *tx)
        .await?;
    if roots.is_empty() {
        tx.rollback().await?;
        return Ok(Restored {
            roots: 0,
            used: used_in(pg, user_id).await?,
        });
    }
    let row = sqlx::query_file!(
        "queries/chat/media/restore_rows.sql",
        user_id,
        &roots,
        signed(QUOTA_BYTES),
        seconds(ATTACH_WINDOW)
    )
    .fetch_one(&mut *tx)
    .await?;
    if row.files == 0 {
        tx.rollback().await?;
        return Ok(Restored {
            roots: 0,
            used: used_in(pg, user_id).await?,
        });
    }
    let (Some(bytes), Some(reserved)) = (row.bytes, row.reserved) else {
        tx.rollback().await?;
        return Err(ChatError::QuotaExceeded {
            used: used_in(pg, user_id).await?.total(),
            quota: QUOTA_BYTES,
        });
    };
    tx.commit().await?;
    Ok(Restored {
        roots: u32::try_from(row.restored).unwrap_or(0),
        used: Used::of(bytes, reserved),
    })
}

#[cfg(test)]
#[path = "../tests/media_race_tests.rs"]
mod race_tests;
