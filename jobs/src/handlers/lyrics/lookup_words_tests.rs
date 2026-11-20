use catalog_sources::LyricsCandidate;
use sqlx::PgPool;

use super::{PLAIN_BODY, candidate, insert_track, lookup_job, offline_handler, settle};

const LINES: &str = "[00:01.00]first invented line\n[00:04.00]second invented line";
const WORDS: &str = "[00:01.00]<00:01.00>first <00:01.50>invented <00:02.00>line<00:02.40>\n[00:04.00]second invented line";
const RECORD_LINES: &str = "[00:01.50]record line one\n[00:05.00]record line two";

type Row = (
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    bool,
    Option<String>,
);

fn with_words(source: &str, lines: &str, words: &str) -> LyricsCandidate {
    let mut found = candidate(source, Some(lines), PLAIN_BODY);
    found.synced_words = Some(words.to_owned());
    found
}

async fn row(pool: &PgPool) -> anyhow::Result<Row> {
    Ok(sqlx::query_as(
        "SELECT synced_lrc, synced_words, synced_source, content_generation,
                embedded_at IS NULL AND embedding_state IS NULL, plain_text
         FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

async fn mark_embedded(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE lyrics_cache SET embedded_at = now(), embedding_state = 'done' WHERE sc_track_id = '42'",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_new_row_gets_the_lines_and_the_words_together(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", LINES, WORDS),
        false,
    )
    .await?;

    let stored = row(&pool).await?;
    assert_eq!(stored.0.as_deref(), Some(LINES));
    assert_eq!(stored.1.as_deref(), Some(WORDS));
    assert_eq!(stored.2.as_deref(), Some("lrclib"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_plain_only_row_takes_the_lines_and_the_words_together(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;

    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", LINES, WORDS),
        false,
    )
    .await?;

    let stored = row(&pool).await?;
    assert_eq!(stored.0.as_deref(), Some(LINES));
    assert_eq!(stored.1.as_deref(), Some(WORDS));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_lrclib_row_without_words_is_upgraded_in_both_columns(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    settle(
        &pool,
        &handler,
        &job,
        candidate("lrclib", Some(RECORD_LINES), PLAIN_BODY),
        true,
    )
    .await?;
    mark_embedded(&pool).await?;
    let before = row(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", LINES, WORDS),
        true,
    )
    .await?;

    let stored = row(&pool).await?;
    assert_eq!(stored.0.as_deref(), Some(LINES));
    assert_eq!(stored.1.as_deref(), Some(WORDS));
    assert_eq!(stored.2.as_deref(), Some("lrclib"));
    assert_eq!(stored.3, before.3 + 1);
    assert!(stored.4);
    assert_eq!(stored.5, before.5);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_lrclib_row_that_has_words_is_left_alone(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    let other = "[00:02.00]<00:02.00>other <00:02.50>words<00:03.00>";
    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", "[00:02.00]other words", other),
        true,
    )
    .await?;
    mark_embedded(&pool).await?;
    let before = row(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", LINES, WORDS),
        true,
    )
    .await?;

    assert_eq!(row(&pool).await?, before);
    Ok(())
}

async fn words_leave_a_foreign_sync_alone(pool: PgPool, synced_source: &str) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    settle(
        &pool,
        &handler,
        &job,
        candidate("lrclib", Some(RECORD_LINES), PLAIN_BODY),
        true,
    )
    .await?;
    sqlx::query("UPDATE lyrics_cache SET synced_source = $1 WHERE sc_track_id = '42'")
        .bind(synced_source)
        .execute(&pool)
        .await?;
    let before = row(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", LINES, WORDS),
        true,
    )
    .await?;

    assert_eq!(row(&pool).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_word_candidate_never_replaces_a_self_generated_sync(pool: PgPool) -> anyhow::Result<()> {
    words_leave_a_foreign_sync_alone(pool, "self_gen").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_word_candidate_never_replaces_a_musixmatch_sync(pool: PgPool) -> anyhow::Result<()> {
    words_leave_a_foreign_sync_alone(pool, "musixmatch").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn words_that_do_not_belong_to_the_lines_are_dropped(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        with_words("lrclib", LINES, "[00:01.00]<00:01.00>other<00:02.00>"),
        false,
    )
    .await?;

    let stored = row(&pool).await?;
    assert_eq!(stored.0.as_deref(), Some(LINES));
    assert_eq!(stored.1, None);
    Ok(())
}
