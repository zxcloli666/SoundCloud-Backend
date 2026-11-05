use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::HOLD_DAYS;

pub async fn queue_trim(tx: &mut PgConnection, conversation: Uuid) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/media/queue_trim.sql", conversation)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub async fn hold_for_report(
    tx: &mut PgConnection,
    report: Uuid,
    conversation: Uuid,
    seq_from: i64,
    seq_to: i64,
    shown: &[Uuid],
) -> ChatResult<Vec<Uuid>> {
    let roots = sqlx::query_file_scalar!(
        "queries/chat/media/hold_roots.sql",
        conversation,
        seq_from,
        seq_to,
        shown
    )
    .fetch_all(&mut *tx)
    .await?;
    hold(tx, report, &roots).await
}

pub async fn hold(tx: &mut PgConnection, report: Uuid, roots: &[Uuid]) -> ChatResult<Vec<Uuid>> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    ledger::lock(tx, roots).await?;
    Ok(
        sqlx::query_file_scalar!("queries/chat/media/hold.sql", roots, report, HOLD_DAYS)
            .fetch_all(&mut *tx)
            .await?,
    )
}

pub async fn release_holds(tx: &mut PgConnection, report: Uuid) -> ChatResult<u64> {
    let roots = sqlx::query_file_scalar!("queries/chat/media/release_holds.sql", report)
        .fetch_all(&mut *tx)
        .await?;
    ledger::settle(tx, &roots).await
}

pub async fn drop_for_messages(
    tx: &mut PgConnection,
    conversation: Uuid,
    seqs: &[i64],
) -> ChatResult<u64> {
    if seqs.is_empty() {
        return Ok(0);
    }
    let roots = sqlx::query_file_scalar!("queries/chat/media/drop_refs.sql", conversation, seqs)
        .fetch_all(&mut *tx)
        .await?;
    ledger::settle(tx, &roots).await
}

#[cfg(test)]
#[path = "../tests/media_hooks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/media_floor_tests.rs"]
mod floor_tests;

#[cfg(test)]
#[path = "../tests/media_hold_tests.rs"]
mod hold_tests;
