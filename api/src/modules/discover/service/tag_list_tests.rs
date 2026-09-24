use serde_json::Value;
use sqlx::PgPool;

const TAG_LIST: &str = include_str!("../../../../queries/discover/service/compute_tag_list.sql");

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, tags)
         SELECT gen_random_uuid(), 'a' || n, 'a' || n, 'test', ARRAY['tag' || (n % 5000)]
         FROM generate_series(1, 20000) AS n",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO discover_tag_counts (tag, artist_count)
         SELECT 'tag' || n, 1 + (n * 7919) % 400 FROM generate_series(1, 5000) AS n",
    )
    .execute(pool)
    .await?;
    sqlx::query("ANALYZE artists, discover_tag_counts")
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_tag_list_reads_the_aggregate_instead_of_the_artist_table(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;

    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {TAG_LIST}"))
        .bind(32_i64)
        .fetch_one(&pool)
        .await?;
    let plan = plan.to_string();

    assert!(
        !plan.contains("\"artists\""),
        "the tag list must not read the artist table on a request: {plan}"
    );
    assert!(
        plan.contains("discover_tag_counts_rank_idx"),
        "the tag list must walk the aggregate index: {plan}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_tag_list_returns_the_widest_tags_first(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO discover_tag_counts (tag, artist_count)
         VALUES ('house', 30), ('techno', 30), ('ambient', 70)",
    )
    .execute(&pool)
    .await?;

    let rows: Vec<(String, i64)> = sqlx::query_as(TAG_LIST)
        .bind(2_i64)
        .fetch_all(&pool)
        .await?;

    assert_eq!(
        rows,
        vec![("ambient".to_owned(), 70), ("house".to_owned(), 30)]
    );
    Ok(())
}
