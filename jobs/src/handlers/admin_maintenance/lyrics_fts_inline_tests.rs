use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::LYRICS_FTS_INLINE;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob};

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_out_of_line_fts_moves_into_the_row_and_stays_the_same(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("ALTER TABLE lyrics_cache ALTER COLUMN fts SET STORAGE EXTENDED")
        .execute(&pool)
        .await?;
    cache(&pool, "l1", &long_lyrics(1)).await?;
    cache(&pool, "l2", &long_lyrics(2)).await?;
    cache(&pool, "s1", "short line of the song\nsecond short line").await?;
    sqlx::query("ALTER TABLE lyrics_cache ALTER COLUMN fts SET STORAGE MAIN")
        .execute(&pool)
        .await?;
    assert_eq!(out_of_line(&pool).await?, ["l1", "l2"]);
    let before = snapshot(&pool).await?;

    let job = claimed(&pool).await?;
    run(&pool, &job).await?;

    assert!(out_of_line(&pool).await?.is_empty());
    assert_eq!(snapshot(&pool).await?, before);
    let totals: (i64, i64) =
        sqlx::query_as("SELECT scanned, changed FROM admin_maintenance_runs WHERE kind = $1")
            .bind(LYRICS_FTS_INLINE)
            .fetch_one(&pool)
            .await?;
    assert_eq!(totals, (3, 2));

    run(&pool, &job).await?;
    assert_eq!(snapshot(&pool).await?, before);
    Ok(())
}

fn long_lyrics(seed: usize) -> String {
    (0..300)
        .map(|word| {
            format!(
                "verse{word}x{} line",
                (word * 7_919 + seed * 104_729) % 100_003
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn cache(pool: &PgPool, id: &str, text: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, source, plain_source, plain_text, fts)
         VALUES ($1, 'lrclib', 'lrclib', $2, to_tsvector('simple', $2))",
    )
    .bind(id)
    .bind(text)
    .execute(pool)
    .await?;
    Ok(())
}

async fn out_of_line(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT sc_track_id FROM lyrics_cache
         WHERE pg_column_toast_chunk_id(fts) IS NOT NULL
         ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn snapshot(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT cache::text FROM lyrics_cache AS cache ORDER BY cache.sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn claimed(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-fts-inline-admin-test".to_owned());
    queue
        .enqueue_if_absent(&NewJob {
            id: Uuid::now_v7(),
            kind: JobKind::AdminLyricsFtsInline,
            dedup_key: Some(LYRICS_FTS_INLINE.to_owned()),
            payload: serde_json::json!({
                "version": "1",
                "payload": { "run_id": Uuid::now_v7() }
            }),
            priority: 20,
            max_attempts: 8,
            available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
        })
        .await?;
    let job = queue
        .claim(
            &[JobKind::AdminLyricsFtsInline],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lyrics fts inline job was not claimed"))?;
    Ok(job)
}

async fn run(pool: &PgPool, job: &LeasedJob) -> anyhow::Result<()> {
    let run_id = Uuid::now_v7();
    let handler = handler(pool);
    for _ in 0..10 {
        handler
            .inline_lyrics_fts(job, AdminMaintenancePayload { run_id })
            .await?;
        let status: String =
            sqlx::query_scalar("SELECT status FROM admin_maintenance_runs WHERE kind = $1")
                .bind(LYRICS_FTS_INLINE)
                .fetch_one(pool)
                .await?;
        if status == "completed" {
            return Ok(());
        }
    }
    anyhow::bail!("the lyrics fts inline run did not complete")
}
