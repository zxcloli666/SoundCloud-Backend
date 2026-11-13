use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::LYRICS_DESCRIPTION;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob};

const MARKER: &str = "… Read More\u{a0}";
const OLD: &str = "2026-09-01 00:00:00";

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct Row {
    sc_track_id: String,
    plain_text: Option<String>,
    content_generation: i64,
    embedded: bool,
    embedding_state: Option<String>,
    updated_at: String,
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_leaked_description_is_cut_off_and_the_old_text_is_kept(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lyrics = "first line of the song\nsecond line of the song";
    let long_description = format!("{}\nsecond description line {MARKER}", "d".repeat(300));

    cache(
        &pool,
        "s1",
        "genius",
        None,
        &format!("short description {MARKER}{lyrics}"),
        OLD,
    )
    .await?;
    cache(
        &pool,
        "s2",
        "genius",
        Some("genius"),
        &format!("{long_description}[Текст песни Artist - Song]\n{lyrics}"),
        OLD,
    )
    .await?;
    cache(&pool, "n1", "genius", None, lyrics, OLD).await?;
    cache(
        &pool,
        "n2",
        "genius",
        None,
        &format!("{}{MARKER}{lyrics}", "x".repeat(1_600)),
        OLD,
    )
    .await?;
    cache(
        &pool,
        "n3",
        "genius",
        None,
        &format!("description {MARKER}{lyrics}"),
        "2026-09-25 00:00:00",
    )
    .await?;
    cache(
        &pool,
        "n4",
        "lrclib",
        None,
        &format!("description {MARKER}{lyrics}"),
        OLD,
    )
    .await?;
    cache(
        &pool,
        "n5",
        "genius",
        Some("lrclib"),
        &format!("description {MARKER}{lyrics}"),
        OLD,
    )
    .await?;
    cache(
        &pool,
        "n6",
        "genius",
        None,
        &format!("description {MARKER}12345678901234567890"),
        OLD,
    )
    .await?;
    cache(
        &pool,
        "s3",
        "genius",
        None,
        &format!("description {MARKER}123456789012345678901"),
        OLD,
    )
    .await?;
    sqlx::query("UPDATE lyrics_cache SET embedded_at = now(), embedding_state = 'done'")
        .execute(&pool)
        .await?;
    let before = rows(&pool).await?;

    let job = claimed(&pool).await?;
    run(&pool, &job).await?;

    let after = rows(&pool).await?;
    for (before, after) in before.iter().zip(&after) {
        match after.sc_track_id.as_str() {
            "s1" | "s2" | "s3" => {
                let want = if after.sc_track_id == "s3" {
                    "123456789012345678901"
                } else {
                    lyrics
                };
                assert_eq!(after.plain_text.as_deref(), Some(want));
                assert_eq!(after.content_generation, before.content_generation + 1);
                assert!(!after.embedded);
                assert_eq!(after.embedding_state, None);
            }
            _ => assert_eq!(after, before),
        }
    }

    let ledger: Vec<(String, String, String, String, bool)> = sqlx::query_as(
        "SELECT sc_track_id, kind, status, outcome, old_plain_text IS NOT NULL
         FROM lyrics_repair
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        ledger,
        [
            (
                "s1".to_owned(),
                "description".to_owned(),
                "done".to_owned(),
                "stripped".to_owned(),
                true
            ),
            (
                "s2".to_owned(),
                "description".to_owned(),
                "done".to_owned(),
                "stripped".to_owned(),
                true
            ),
            (
                "s3".to_owned(),
                "description".to_owned(),
                "done".to_owned(),
                "stripped".to_owned(),
                true
            ),
        ]
    );
    let old: Vec<String> =
        sqlx::query_scalar("SELECT old_plain_text FROM lyrics_repair ORDER BY sc_track_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(old[0], format!("short description {MARKER}{lyrics}"));
    assert_eq!(
        old[1],
        format!("{long_description}[Текст песни Artist - Song]\n{lyrics}")
    );

    let totals: (i64, i64) =
        sqlx::query_as("SELECT scanned, changed FROM admin_maintenance_runs WHERE kind = $1")
            .bind(LYRICS_DESCRIPTION)
            .fetch_one(&pool)
            .await?;
    assert_eq!(totals, (9, 3));

    let first = snapshot(&pool).await?;
    run(&pool, &job).await?;
    assert_eq!(first, snapshot(&pool).await?);
    Ok(())
}

async fn claimed(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-description-admin-test".to_owned());
    queue
        .enqueue_if_absent(&NewJob {
            id: Uuid::now_v7(),
            kind: JobKind::AdminLyricsDescription,
            dedup_key: Some(LYRICS_DESCRIPTION.to_owned()),
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
            &[JobKind::AdminLyricsDescription],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lyrics description job was not claimed"))?;
    Ok(job)
}

async fn run(pool: &PgPool, job: &LeasedJob) -> anyhow::Result<()> {
    let run_id = Uuid::now_v7();
    let handler = handler(pool);
    for _ in 0..10 {
        handler
            .strip_lyrics_descriptions(job, AdminMaintenancePayload { run_id })
            .await?;
        let status: String =
            sqlx::query_scalar("SELECT status FROM admin_maintenance_runs WHERE kind = $1")
                .bind(LYRICS_DESCRIPTION)
                .fetch_one(pool)
                .await?;
        if status == "completed" {
            return Ok(());
        }
    }
    anyhow::bail!("the lyrics description run did not complete")
}

async fn rows(pool: &PgPool) -> anyhow::Result<Vec<Row>> {
    Ok(sqlx::query_as(
        "SELECT sc_track_id, plain_text, content_generation, embedded_at IS NOT NULL AS embedded, embedding_state, updated_at::text AS updated_at
         FROM lyrics_cache
         ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn snapshot(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT cache::text FROM lyrics_cache AS cache
         UNION ALL
         SELECT repair::text FROM lyrics_repair AS repair
         ORDER BY 1",
    )
    .fetch_all(pool)
    .await?)
}

async fn cache(
    pool: &PgPool,
    id: &str,
    source: &str,
    plain_source: Option<&str>,
    text: &str,
    created_at: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, source, plain_source, plain_text, created_at)
         VALUES ($1, $2, $3, $4, $5::text::timestamp)",
    )
    .bind(id)
    .bind(source)
    .bind(plain_source)
    .bind(text)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(())
}
