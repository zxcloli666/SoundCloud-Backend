use std::time::Duration;

use backend_contracts::{
    JobLane, LIBRARY_READ_KINDS, LIBRARY_READ_PRIORITY, OWNER_COLLECTION_PRIORITY,
};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

const PUBLISH_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) async fn publish_library_waits(
    pool: PgPool,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let mut interval = tokio::time::interval(PUBLISH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            _ = interval.tick() => {
                if let Err(error) = publish_once(&pool).await {
                    tracing::warn!(%error, "library queue waits were not published");
                }
            }
        }
    }
}

async fn publish_once(pool: &PgPool) -> Result<(), sqlx::Error> {
    let kinds: Vec<String> = LIBRARY_READ_KINDS
        .iter()
        .map(|kind| kind.as_str().to_owned())
        .collect();
    let waits = sqlx::query_file!(
        "queries/queue/publish_library_waits.sql",
        &kinds,
        JobLane::CoreBulk.as_str(),
        LIBRARY_READ_PRIORITY,
        OWNER_COLLECTION_PRIORITY
    )
    .fetch_all(pool)
    .await?;
    for wait in waits {
        crate::metrics::record_library_wait(&wait.kind, &wait.lane, wait.due, wait.waiting_seconds);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::publish_once;

    #[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
    async fn the_waits_count_due_library_reads_per_lane(pool: PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, available_at)
             VALUES (gen_random_uuid(), 'catalog.collection', 'core_bulk', 'a', '{}', 40, now() - interval '9 seconds'),
                    (gen_random_uuid(), 'catalog.collection', 'core_bulk', 'b', '{}', 20, now() - interval '30 minutes'),
                    (gen_random_uuid(), 'catalog.collection', 'core_bulk', 'c', '{}', 25, now() + interval '1 minute'),
                    (gen_random_uuid(), 'catalog.collection', 'core_bulk', 'd', '{}', 15, now() - interval '2 hours'),
                    (gen_random_uuid(), 'playlists.observe_shadow', 'core_bulk', 'e', '{}', 0, now() - interval '1 day')",
        )
        .execute(&pool)
        .await?;

        publish_once(&pool).await?;
        publish_once(&pool).await?;

        let waits: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT kind, lane, due, coalesce(extract(epoch FROM now() - oldest_due_at), 0)::bigint
             FROM job_queue_waits ORDER BY kind, lane",
        )
        .fetch_all(&pool)
        .await?;
        assert_eq!(
            waits,
            [
                ("catalog.collection".into(), "queue".into(), 1, 1800),
                ("catalog.collection".into(), "read".into(), 1, 9),
                ("playlists.observe_shadow".into(), "queue".into(), 0, 0),
                ("playlists.observe_shadow".into(), "read".into(), 0, 0),
            ]
        );
        Ok(())
    }
}
