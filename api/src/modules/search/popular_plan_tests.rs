use serde_json::Value;
use sqlx::PgPool;

const GLOBAL_SEARCH: &str =
    include_str!("../../../queries/search/repository/search_tracks_global.sql");

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, sharing, play_count_sc
         )
         SELECT n::text, 'soundcloud:tracks:' || n,
                (ARRAY['ocean drift', 'silent road', 'ocean road'])[1 + n % 3] || ' ' || n,
                (ARRAY['ocean drift', 'silent road', 'ocean road'])[1 + n % 3] || ' ' || n,
                120000, 'public', n
         FROM generate_series(1, 60000) AS n;
         ANALYZE tracks;",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn plan(pool: &PgPool, needle: &str) -> anyhow::Result<String> {
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {GLOBAL_SEARCH}"))
        .bind(needle)
        .bind(30_i64)
        .bind(0_i64)
        .bind(needle)
        .bind(Option::<Vec<String>>::None)
        .bind(Option::<String>::None)
        .bind(Option::<Vec<String>>::None)
        .bind(false)
        .fetch_one(pool)
        .await?;
    Ok(plan.to_string())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_common_word_walks_the_popularity_index_instead_of_sorting_every_match(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;

    let plan = plan(&pool, "%ocean%").await?;

    assert!(
        plan.contains("tracks_public_popular_idx"),
        "a page of the most played must come from the popularity index: {plan}"
    );
    assert!(
        !plan.contains("\"Sort\""),
        "a common word must not sort every matching track: {plan}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rare_word_still_goes_through_the_trigram_index(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;

    let plan = plan(&pool, "%drift 4242%").await?;

    assert!(
        plan.contains("trgm"),
        "a rare word must be found by trigram, not by walking popularity: {plan}"
    );
    Ok(())
}
