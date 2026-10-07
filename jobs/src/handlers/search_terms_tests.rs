use sqlx::PgPool;

use super::*;

async fn insert_tracks(pool: &PgPool, tracks: &[(&str, &str, &str)]) -> anyhow::Result<()> {
    for (id, title, uploader) in tracks {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms, uploader_username)
             VALUES ($1, 'soundcloud:tracks:' || $1, $2, lower($2), 'public', 1000, $3)",
        )
        .bind(id)
        .bind(title)
        .bind(uploader)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn lexicon(pool: &PgPool) -> anyhow::Result<Vec<(String, i32)>> {
    Ok(
        sqlx::query_as("SELECT word, ndoc FROM search_terms ORDER BY word")
            .fetch_all(pool)
            .await?,
    )
}

async fn row_versions(pool: &PgPool) -> anyhow::Result<Vec<(String, String, String, String)>> {
    Ok(sqlx::query_as(
        "SELECT word, xmin::text, xmax::text, ctid::text FROM search_terms ORDER BY word",
    )
    .fetch_all(pool)
    .await?)
}

async fn refresh_with(pool: &PgPool, chunk_rows: i64) -> anyhow::Result<Option<Refresh>> {
    let mut connection = pool.acquire().await?;
    connection.close_on_drop();
    Ok(refresh(&mut connection, chunk_rows).await?)
}

fn words(expected: &[(&str, i32)]) -> Vec<(String, i32)> {
    expected
        .iter()
        .map(|(word, ndoc)| ((*word).to_owned(), *ndoc))
        .collect()
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_search_lexicon_learns_catalog_words_but_not_whisper_gibberish(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms, uploader_username)
         VALUES ('1', 'soundcloud:tracks:1', 'Umbrella', 'umbrella', 'public', 1000, 'rihanna'),
                ('2', 'soundcloud:tracks:2', 'Umbrella', 'umbrella', 'public', 1000, 'rihanna'),
                ('3', 'soundcloud:tracks:3', 'Hidden', 'hidden', 'private', 1000, 'rihanna');
         INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('1', 'zzgibber', 'self_gen'), ('2', 'zzgibber', 'self_gen')",
    )
    .execute(&pool)
    .await?;

    SearchTermsHandler::new(pool.clone())
        .refresh()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    assert_eq!(
        lexicon(&pool).await?,
        words(&[("rihanna", 2), ("umbrella", 2)])
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_removed_track_drops_its_stale_word_and_lowers_the_shared_counts(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_tracks(
        &pool,
        &[
            ("1", "Umbrella", "rihanna"),
            ("2", "Umbrella", "rihanna"),
            ("3", "Diamonds", "rihanna"),
            ("4", "Diamonds", "rihanna"),
        ],
    )
    .await?;
    refresh_with(&pool, CHUNK_ROWS).await?;
    assert_eq!(
        lexicon(&pool).await?,
        words(&[("diamonds", 2), ("rihanna", 4), ("umbrella", 2)])
    );
    let untouched = row_versions(&pool).await?;

    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '4'")
        .execute(&pool)
        .await?;
    let second = refresh_with(&pool, CHUNK_ROWS).await?;

    assert_eq!(
        lexicon(&pool).await?,
        words(&[("rihanna", 3), ("umbrella", 2)])
    );
    assert_eq!(
        second,
        Some(Refresh {
            changed: 2,
            chunks: 1,
            upserted: 1,
            removed: 1,
        })
    );
    let umbrella = |rows: &[(String, String, String, String)]| {
        rows.iter().find(|row| row.0 == "umbrella").cloned()
    };
    assert_eq!(
        umbrella(&row_versions(&pool).await?),
        umbrella(&untouched),
        "a word whose count did not change keeps its row version, so it never replicates"
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_unchanged_catalog_rewrites_and_locks_no_lexicon_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_tracks(
        &pool,
        &[
            ("1", "Umbrella", "rihanna"),
            ("2", "Umbrella", "rihanna"),
            ("3", "Кукла колдуна", "kish"),
            ("4", "Кукла колдуна", "kish"),
        ],
    )
    .await?;
    refresh_with(&pool, CHUNK_ROWS).await?;
    let before = row_versions(&pool).await?;

    let second = refresh_with(&pool, CHUNK_ROWS).await?;

    assert_eq!(second, Some(Refresh::default()));
    assert_eq!(row_versions(&pool).await?, before);
    assert!(
        before.iter().all(|row| row.2 == "0"),
        "no row may even be locked, a lock dirties the page and writes WAL: {before:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn every_chunk_commits_on_its_own_and_the_result_matches_one_pass(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_tracks(
        &pool,
        &[
            ("1", "Umbrella", "rihanna"),
            ("2", "Umbrella", "rihanna"),
            ("3", "Diamonds", "rihanna"),
            ("4", "Diamonds", "rihanna"),
            ("5", "Stay", "rihanna"),
            ("6", "Stay", "rihanna"),
        ],
    )
    .await?;

    let chunked = refresh_with(&pool, 2).await?;

    assert_eq!(
        chunked,
        Some(Refresh {
            changed: 4,
            chunks: 2,
            upserted: 4,
            removed: 0,
        })
    );
    let transactions: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT xmin::text) FROM search_terms")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        transactions, 2,
        "each chunk must commit separately so the subscriber never applies one huge transaction"
    );
    let chunked_lexicon = lexicon(&pool).await?;

    sqlx::query("TRUNCATE search_terms").execute(&pool).await?;
    refresh_with(&pool, CHUNK_ROWS).await?;
    assert_eq!(lexicon(&pool).await?, chunked_lexicon);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_second_refresh_steps_aside_while_the_first_holds_the_lock(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_tracks(
        &pool,
        &[("1", "Umbrella", "rihanna"), ("2", "Umbrella", "rihanna")],
    )
    .await?;
    let mut running = pool.acquire().await?;
    let acquired = sqlx::query_file_scalar!("queries/search/lock_terms.sql")
        .fetch_one(&mut *running)
        .await?;
    assert!(acquired);

    assert_eq!(refresh_with(&pool, CHUNK_ROWS).await?, None);
    SearchTermsHandler::new(pool.clone())
        .refresh()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    assert!(lexicon(&pool).await?.is_empty());

    running.close().await?;
    refresh_with(&pool, CHUNK_ROWS).await?;
    assert_eq!(
        lexicon(&pool).await?,
        words(&[("rihanna", 2), ("umbrella", 2)])
    );
    Ok(())
}
