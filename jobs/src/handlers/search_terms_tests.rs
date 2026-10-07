use sqlx::PgPool;

use super::*;

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

    let words: Vec<(String, i32)> =
        sqlx::query_as("SELECT word, ndoc FROM search_terms ORDER BY word")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        words,
        vec![("rihanna".to_owned(), 2), ("umbrella".to_owned(), 2)]
    );
    Ok(())
}
