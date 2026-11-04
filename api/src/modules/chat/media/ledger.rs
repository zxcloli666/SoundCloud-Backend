use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::ChatResult;

pub async fn lock(tx: &mut PgConnection, ids: &[Uuid]) -> ChatResult<Vec<Uuid>> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/media/lock_roots.sql", ids)
            .fetch_all(&mut *tx)
            .await?,
    )
}

pub async fn settle(tx: &mut PgConnection, ids: &[Uuid]) -> ChatResult<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    lock(tx, ids).await?;
    let settled = sqlx::query_file_scalar!("queries/chat/media/settle.sql", ids)
        .fetch_one(&mut *tx)
        .await?;
    Ok(u64::try_from(settled).unwrap_or(0))
}
