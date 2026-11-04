use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::HOLD_DAYS;

pub async fn queue_trim(executor: impl PgExecutor<'_>, conversation: Uuid) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/media/queue_trim.sql", conversation)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn hold_for_report(
    tx: &mut PgConnection,
    report: Uuid,
    conversation: Uuid,
    seq_from: i64,
    seq_to: i64,
) -> ChatResult<u64> {
    let roots = sqlx::query_file_scalar!(
        "queries/chat/media/hold_roots.sql",
        conversation,
        seq_from,
        seq_to
    )
    .fetch_all(&mut *tx)
    .await?;
    if roots.is_empty() {
        return Ok(0);
    }
    ledger::lock(tx, &roots).await?;
    let held = sqlx::query_file!(
        "queries/chat/media/hold_range.sql",
        &roots,
        report,
        HOLD_DAYS
    )
    .execute(&mut *tx)
    .await?;
    Ok(held.rows_affected())
}

pub async fn release_holds(tx: &mut PgConnection, report: Uuid) -> ChatResult<u64> {
    let mut roots = sqlx::query_file_scalar!("queries/chat/media/release_holds.sql", report)
        .fetch_all(&mut *tx)
        .await?;
    roots.sort_unstable();
    roots.dedup();
    ledger::settle(tx, &roots).await
}

#[cfg(test)]
#[path = "../tests/media_hooks_tests.rs"]
mod tests;
