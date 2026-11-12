use sqlx::PgPool;

use super::catalog_tests::service;
use super::resolve::{Kind, Resolved, keep_on_timeout};
use crate::error::AppError;

const KNOWN: &str = "00000000-0000-0000-0000-0000000000b1";
const LISTENED: &str = "00000000-0000-0000-0000-0000000000b2";

async fn seed_equal_matches(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary,
                              monthly_listeners, popularity_score)
         VALUES ($1::uuid, 'Kino Alfa', 'kino alfa', 'test', 500, 3, 0.82),
                ($2::uuid, 'Kino Beta', 'kino beta', 'test', 4, 5000, 0.11)",
    )
    .bind(KNOWN)
    .bind(LISTENED)
    .execute(pool)
    .await?;
    super::lexicon_refresh::refresh(pool).await
}

async fn artists(pool: &PgPool, said: &str) -> anyhow::Result<Vec<Resolved>> {
    let found = service(pool)?
        .resolve(&[said.to_owned()], &[Kind::Artist], 5)
        .await?;
    Ok(found.get(&Kind::Artist).cloned().unwrap_or_default())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_artist_carries_the_popularity_of_its_catalog(pool: PgPool) -> anyhow::Result<()> {
    seed_equal_matches(&pool).await?;
    let found = artists(&pool, "kino").await?;
    let known = found.iter().find(|artist| artist.id == KNOWN);
    let popularity = known.map(|artist| artist.popularity).unwrap_or_default();
    assert!((popularity - 0.82).abs() < 0.001, "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn of_two_equal_matches_the_one_with_the_bigger_catalog_leads(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_equal_matches(&pool).await?;
    let found = artists(&pool, "kino").await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [KNOWN, LISTENED]);
    Ok(())
}

async fn failure_of(pool: &PgPool, sql: &str) -> anyhow::Result<AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('statement_timeout', '10', true)")
        .execute(&mut *tx)
        .await?;
    let error = sqlx::query(sql)
        .execute(&mut *tx)
        .await
        .expect_err("the statement fails");
    Ok(AppError::from(error))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_later_variant_cut_by_the_statement_timeout_keeps_what_was_found(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_equal_matches(&pool).await?;
    let found = artists(&pool, "kino").await?;
    assert!(!found.is_empty());

    let slow = failure_of(&pool, "SELECT pg_sleep(1)").await?;
    let kept = keep_on_timeout(found.clone(), slow)?;
    assert_eq!(kept, found);

    let slow = failure_of(&pool, "SELECT pg_sleep(1)").await?;
    let nothing = keep_on_timeout(Vec::new(), slow).expect_err("nothing found yet");
    assert_eq!(nothing.public_code(), "search_timeout");

    let broken = failure_of(&pool, "SELECT 1 / 0").await?;
    let broken = keep_on_timeout(found, broken).expect_err("not a timeout");
    assert_ne!(broken.public_code(), "search_timeout");
    Ok(())
}
