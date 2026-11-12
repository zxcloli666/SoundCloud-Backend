use sqlx::PgPool;

use super::catalog_tests::service;
use super::resolve::{Kind, Resolved, keep_on_timeout};
use crate::error::AppError;

const KNOWN: &str = "00000000-0000-0000-0000-0000000000b1";
const LISTENED: &str = "00000000-0000-0000-0000-0000000000b2";
const SPELLED: &str = "00000000-0000-0000-0000-0000000000b3";
const NEAR: &str = "00000000-0000-0000-0000-0000000000b4";
const LATIN: &str = "00000000-0000-0000-0000-0000000000b5";
const CYRILLIC: &str = "00000000-0000-0000-0000-0000000000b6";
const PART: &str = "00000000-0000-0000-0000-0000000000b7";

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

async fn seed_a_name_said_in_cyrillic(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary,
                              monthly_listeners, popularity_score)
         VALUES ($1::uuid, 'Psychosis', 'psychosis', 'test', 40, 900, 0.4),
                ($2::uuid, 'Психоз', 'психоз', 'test', 60, 2000, 0.6)",
    )
    .bind(SPELLED)
    .bind(NEAR)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO search_terms (word, ndoc, latin)
         SELECT word, ndoc, search_latin(word)
         FROM (VALUES ('psychosis', 2107), ('психоз', 262)) AS v (word, ndoc)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_a_name_of_two_words(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary,
                              monthly_listeners, popularity_score)
         VALUES ($1::uuid, 'Playboi Carti', 'playboi carti', 'test', 40, 900, 0.9),
                ($2::uuid, 'Плейбой Карти', 'плеибои карти', 'test', 40, 900, 0.2),
                ($3::uuid, 'Плейбой Клуб', 'плеибои клуб', 'test', 40, 900, 0.5)",
    )
    .bind(LATIN)
    .bind(CYRILLIC)
    .bind(PART)
    .execute(pool)
    .await?;
    words(pool, "('плеибои', 73), ('карти', 156), ('клуб', 5000)").await
}

async fn words(pool: &PgPool, counted: &str) -> anyhow::Result<()> {
    sqlx::query(&format!(
        "INSERT INTO search_terms (word, ndoc, latin)
         SELECT word, ndoc, search_latin(word) FROM (VALUES {counted}) AS v (word, ndoc)"
    ))
    .execute(pool)
    .await?;
    Ok(())
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_name_said_in_cyrillic_finds_the_artist_spelled_in_latin(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_said_in_cyrillic(&pool).await?;
    let found = artists(&pool, "психосиз").await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [SPELLED, NEAR], "{found:?}");
    assert_eq!(found[0].variant, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_name_with_a_latin_spelling_is_not_looked_up_by_its_parts(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_of_two_words(&pool).await?;
    words(&pool, "('playboi', 17698), ('carti', 21025)").await?;
    let found = artists(&pool, "плейбой карти").await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [LATIN, CYRILLIC], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_name_without_a_latin_spelling_is_looked_up_by_its_parts_too(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_of_two_words(&pool).await?;
    let found = artists(&pool, "плейбой карти").await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [CYRILLIC, PART], "{found:?}");
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
