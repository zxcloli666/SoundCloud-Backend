use backend_contracts::pipeline::LyricsEmbeddingRequest;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::{Step, acknowledge, prepare, release};

async fn seed_lyrics(
    pool: &PgPool,
    plain_text: Option<&str>,
    synced_lrc: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (
             sc_track_id, plain_text, synced_lrc, source, language, embedding_state
         ) VALUES ('42', $1, $2, 'genius', 'EN', 'queued')",
    )
    .bind(plain_text)
    .bind(synced_lrc)
    .execute(pool)
    .await?;
    Ok(())
}

async fn published(pool: &PgPool) -> anyhow::Result<LyricsEmbeddingRequest> {
    match prepare(pool, "42").await {
        Ok(Step::Publish(request)) => Ok(request),
        Ok(_) => anyhow::bail!("no request was opened"),
        Err(error) => anyhow::bail!("{error}"),
    }
}

async fn cache_state(pool: &PgPool) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT embedding_state FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_queued_row_opens_one_request_over_text_without_timestamps(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_lyrics(&pool, None, Some("[00:01.00]one\n[00:02.50]two")).await?;

    let request = published(&pool).await?;

    let wire = sqlx::query_as::<_, (String, String, Vec<u8>, Option<String>, i64)>(
        "SELECT status, request_message_id, request_sha256, request_language,
                lyrics_content_generation
         FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(request.text, "one\ntwo");
    assert_eq!(request.language.as_deref(), Some("en"));
    assert!(request.request_id.starts_with("lyr:42:"));
    assert_eq!(
        wire,
        (
            "pending".to_owned(),
            request.request_id.clone(),
            Sha256::digest(b"one\ntwo").to_vec(),
            Some("en".to_owned()),
            1
        )
    );
    assert_eq!(cache_state(&pool).await?.as_deref(), Some("pending"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_unacknowledged_request_is_published_again_under_the_same_identity(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_lyrics(&pool, Some("first line\nsecond line"), None).await?;

    let first = published(&pool).await?;
    let retry = published(&pool).await?;
    acknowledge(&pool, "42", &retry.request_id)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let after_acknowledgement = prepare(&pool, "42")
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let acknowledged: bool = sqlx::query_scalar(
        "SELECT publish_acknowledged_at IS NOT NULL
         FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(first, retry);
    assert!(acknowledged);
    assert_eq!(cache_state(&pool).await?.as_deref(), Some("dispatched"));
    assert!(matches!(after_acknowledgement, Step::Nothing));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn lyrics_that_leave_no_text_are_skipped(pool: PgPool) -> anyhow::Result<()> {
    seed_lyrics(&pool, None, Some("[00:01.00]\n[00:02.00]")).await?;

    let step = prepare(&pool, "42")
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let wire_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_embedding_wire_state")
        .fetch_one(&pool)
        .await?;
    assert!(matches!(step, Step::Nothing));
    assert_eq!(cache_state(&pool).await?.as_deref(), Some("skipped"));
    assert_eq!(wire_rows, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_long_text_is_cut_to_the_wire_limit_on_a_line_boundary(
    pool: PgPool,
) -> anyhow::Result<()> {
    let text = vec!["строка длинной песни"; 1_500].join("\n");
    seed_lyrics(&pool, Some(&text), None).await?;

    let request = published(&pool).await?;

    assert!(request.text.len() <= 16_000);
    assert!(text.starts_with(&request.text));
    assert!(request.text.ends_with("песни"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn reopening_a_request_replaces_it_and_counts_the_reopen(pool: PgPool) -> anyhow::Result<()> {
    seed_lyrics(&pool, Some("first line\nsecond line"), None).await?;
    let first = published(&pool).await?;
    sqlx::raw_sql(
        "UPDATE lyrics_embedding_wire_state
         SET status = 'reopenable', completed_at = now(), result_reason = 'engine_restarted'
         WHERE sc_track_id = '42';
         UPDATE lyrics_cache SET embedding_state = 'queued' WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;

    let second = published(&pool).await?;

    let wire = sqlx::query_as::<_, (String, String, i32, Option<String>)>(
        "SELECT status, request_message_id, reopen_count, result_reason
         FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_ne!(first.request_id, second.request_id);
    assert_eq!(
        wire,
        ("pending".to_owned(), second.request_id.clone(), 1, None)
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn switched_off_dispatch_returns_only_queued_rows_to_the_reaper(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_lyrics(&pool, Some("first line\nsecond line"), None).await?;
    release(&pool, "42")
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let released = cache_state(&pool).await?;
    sqlx::query("UPDATE lyrics_cache SET embedding_state = 'queued' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    published(&pool).await?;

    release(&pool, "42")
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    assert_eq!(released, None);
    assert_eq!(cache_state(&pool).await?.as_deref(), Some("pending"));
    Ok(())
}
