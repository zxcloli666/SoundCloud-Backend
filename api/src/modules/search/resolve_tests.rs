use std::time::Duration;

use futures::future::try_join_all;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::catalog_tests::{seed, service, song};
use super::lanes::IDLE_KEPT;
use super::resolve::{Kind, Resolved, keep_on_timeout};
use crate::error::AppError;

const KNOWN: &str = "00000000-0000-0000-0000-0000000000b1";
const LISTENED: &str = "00000000-0000-0000-0000-0000000000b2";
const SPELLED: &str = "00000000-0000-0000-0000-0000000000b3";
const NEAR: &str = "00000000-0000-0000-0000-0000000000b4";
const LATIN: &str = "00000000-0000-0000-0000-0000000000b5";
const CYRILLIC: &str = "00000000-0000-0000-0000-0000000000b6";
const PART: &str = "00000000-0000-0000-0000-0000000000b7";
const WHOLE: &str = "00000000-0000-0000-0000-0000000000b8";
const FORM: &str = "00000000-0000-0000-0000-0000000000b9";

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

async fn seed_two_names_sharing_a_word(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary,
                              monthly_listeners, popularity_score)
         VALUES ($1::uuid, 'Playboi Carti', 'playboi carti', 'test', 40, 900, 0.3),
                ($2::uuid, 'Carti Club', 'carti club', 'test', 40, 900, 0.5)",
    )
    .bind(LATIN)
    .bind(PART)
    .execute(pool)
    .await?;
    words(pool, "('playboi', 17698), ('carti', 21025), ('club', 5000)").await
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

async fn seed_a_name_and_its_other_form(pool: &PgPool, popularity: f64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary,
                              monthly_listeners, popularity_score)
         VALUES ($1::uuid, 'Queen', 'queen', 'test', 40, 900, $3),
                ($2::uuid, 'Queens', 'queens', 'test', 40, 900, 0.5)",
    )
    .bind(WHOLE)
    .bind(FORM)
    .bind(popularity)
    .execute(pool)
    .await?;
    Ok(())
}

async fn artists(pool: &PgPool, said: &str) -> anyhow::Result<Vec<Resolved>> {
    artists_of(pool, &[said]).await
}

async fn artists_of(pool: &PgPool, said: &[&str]) -> anyhow::Result<Vec<Resolved>> {
    found_of(pool, &[Kind::Artist], said).await
}

async fn found_of(pool: &PgPool, kinds: &[Kind], said: &[&str]) -> anyhow::Result<Vec<Resolved>> {
    let variants: Vec<String> = said.iter().map(|text| (*text).to_owned()).collect();
    let found = service(pool)?.resolve(&variants, kinds, 5, false).await?;
    Ok(found.into_values().flatten().collect())
}

async fn artists_of_a_known_name(pool: &PgPool, said: &[&str]) -> anyhow::Result<Vec<Resolved>> {
    let variants: Vec<String> = said.iter().map(|text| (*text).to_owned()).collect();
    let kinds = [Kind::Artist];
    let found = service(pool)?.resolve(&variants, &kinds, 5, true).await?;
    Ok(found.into_values().flatten().collect())
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_whole_name_found_exactly_and_well_known_ends_the_lookup(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.8).await?;
    let found = artists_of(&pool, &["queen", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [WHOLE], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_whole_name_found_exactly_but_not_well_known_keeps_the_lookup_going(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.79).await?;
    let found = artists_of(&pool, &["queen", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [WHOLE, FORM], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_known_name_found_exactly_ends_the_lookup_whatever_its_popularity(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.3).await?;
    let found = artists_of_a_known_name(&pool, &["queen", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [WHOLE], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_known_name_found_exactly_is_not_looked_up_loosely(pool: PgPool) -> anyhow::Result<()> {
    seed_two_names_sharing_a_word(&pool).await?;
    let found = artists_of(&pool, &["playboi carti"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [LATIN, PART], "{found:?}");
    let found = artists_of_a_known_name(&pool, &["playboi carti"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [LATIN], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_known_name_not_found_exactly_is_looked_up_loosely_too(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_two_names_sharing_a_word(&pool).await?;
    let found = artists_of_a_known_name(&pool, &["carti playboi"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [LATIN, PART], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_known_name_not_found_exactly_keeps_the_lookup_going(pool: PgPool) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.3).await?;
    let found = artists_of_a_known_name(&pool, &["kween", "queen", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [FORM, WHOLE], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_known_name_cleaned_away_makes_no_later_variant_known(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.3).await?;
    let found = artists_of_a_known_name(&pool, &["q", "queen", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [FORM, WHOLE], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_spelling_derived_from_a_known_name_still_has_to_be_well_known(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_said_in_cyrillic(&pool).await?;
    seed_a_name_and_its_other_form(&pool, 0.3).await?;
    let found = artists_of_a_known_name(&pool, &["психосиз", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert!(ids.contains(&SPELLED) && ids.contains(&FORM), "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_later_form_found_exactly_does_not_end_the_lookup(pool: PgPool) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.86).await?;
    let found = artists_of(&pool, &["kween", "queen", "queens"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [WHOLE, FORM], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_latin_spelling_the_request_lists_later_is_asked_first_and_ends_the_lookup(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_of_two_words(&pool).await?;
    words(&pool, "('playboi', 17698), ('carti', 21025)").await?;
    let found = artists_of(&pool, &["плейбой карти", "плейбой", "Playboi Carti"]).await?;
    let ids: Vec<&str> = found.iter().map(|artist| artist.id.as_str()).collect();
    assert_eq!(ids, [LATIN, CYRILLIC], "{found:?}");
    assert_eq!(found[0].variant, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_title_said_with_its_artist_found_exactly_and_well_known_ends_the_lookup(
    pool: PgPool,
) -> anyhow::Result<()> {
    let songs = [
        song(1, "Nightcall", "Kavinsky", 5_000_000),
        song(2, "Roadgame", "Other Band", 100),
    ];
    seed(&pool, &songs).await?;
    let found = found_of(&pool, &[Kind::Track], &["nightcall kavinsky", "roadgame"]).await?;
    assert_eq!(found[0].id, "soundcloud:tracks:1", "{found:?}");
    assert!(found.iter().all(|track| track.variant == 0), "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_name_that_ends_the_lookup_ends_it_for_every_kind_asked(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.86).await?;
    seed(&pool, &[song(2, "Roadgame", "Other Band", 100)]).await?;
    let kinds = [Kind::Artist, Kind::Track];
    let found = found_of(&pool, &kinds, &["queen", "roadgame"]).await?;
    let ids: Vec<&str> = found.iter().map(|hit| hit.id.as_str()).collect();
    assert_eq!(ids, [WHOLE], "{found:?}");
    Ok(())
}

async fn a_pool_with_idle(pool: &PgPool, idle: usize) -> anyhow::Result<PgPool> {
    let own = PgPoolOptions::new()
        .max_connections(8)
        .idle_timeout(None)
        .connect_with((*pool.connect_options()).clone())
        .await?;
    drop(try_join_all((0..idle).map(|_| own.acquire())).await?);
    while own.num_idle() < idle {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(own)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_kind_that_cannot_answer_in_time_does_not_hold_back_the_other_kinds(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.5).await?;
    let mut locked = pool.begin().await?;
    sqlx::query("LOCK TABLE tracks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *locked)
        .await?;
    let own = a_pool_with_idle(&pool, IDLE_KEPT + 2).await?;
    let kinds = [Kind::Track, Kind::Artist];
    let found = service(&own)?
        .resolve(&["queen".to_owned()], &kinds, 5, false)
        .await;
    own.close().await;
    let found = found?;
    let ids: Vec<&str> = found
        .values()
        .flatten()
        .map(|hit| hit.id.as_str())
        .collect();
    assert_eq!(ids, [WHOLE], "{found:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn with_no_connection_to_spare_every_kind_is_still_asked(pool: PgPool) -> anyhow::Result<()> {
    seed_a_name_and_its_other_form(&pool, 0.5).await?;
    seed(&pool, &[song(2, "Queen", "Other Band", 100)]).await?;
    let own = a_pool_with_idle(&pool, IDLE_KEPT).await?;
    let search = service(&own)?;
    let lanes = search.lanes(4).await.map(|lanes| lanes.len());
    let kinds = [Kind::Artist, Kind::Track];
    let found = search
        .resolve(&["queen".to_owned()], &kinds, 5, false)
        .await;
    own.close().await;
    assert_eq!(lanes?, 1);
    let found = found?;
    assert_eq!(found[&Kind::Artist][0].id, WHOLE, "{found:?}");
    assert_eq!(
        found[&Kind::Track][0].id,
        "soundcloud:tracks:2",
        "{found:?}"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_lanes_of_a_lookup_leave_idle_connections_to_the_rest(
    pool: PgPool,
) -> anyhow::Result<()> {
    let own = a_pool_with_idle(&pool, IDLE_KEPT + 2).await?;
    let lanes = service(&own)?.lanes(4).await;
    let idle = own.num_idle();
    let lanes = lanes.map(|lanes| lanes.len());
    own.close().await;
    assert_eq!(lanes?, 2);
    assert_eq!(idle, IDLE_KEPT);
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
