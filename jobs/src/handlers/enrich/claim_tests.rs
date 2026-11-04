use sqlx::PgPool;

use super::Claim;
use crate::handlers::star_priority::{Tier, Turn};

async fn seed_unlinked_tracks(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             index_priority, enrich_next_run_at
         ) VALUES
             ('1', 'soundcloud:tracks:1', 'Oldest', 'oldest', 1000, 5, now() - interval '5 days'),
             ('2', 'soundcloud:tracks:2', 'Liked', 'liked', 1000, 1, now() - interval '4 days'),
             ('3', 'soundcloud:tracks:3', 'Star other', 'star other', 1000, 5, now() - interval '3 days'),
             ('4', 'soundcloud:tracks:4', 'Star liked', 'star liked', 1000, 1, now() - interval '2 days'),
             ('5', 'soundcloud:tracks:5', 'Discovery', 'discovery', 1000, 5, now() - interval '1 day'),
             ('6', 'soundcloud:tracks:6', 'Later', 'later', 1000, 1, now() + interval '1 day');
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('3', 3), ('4', 1), ('6', 1);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn claimed(pool: &PgPool, batch: i64, turn: Turn) -> anyhow::Result<Vec<String>> {
    let claim = Claim {
        batch,
        max_attempts: 8,
    };
    Ok(claim
        .take(pool, turn)
        .await?
        .into_iter()
        .map(|track| track.sc_track_id)
        .collect())
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_star_turn_links_star_likes_then_likes_then_star_tracks_then_the_rest(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_unlinked_tracks(&pool).await?;

    let mut claimed = claimed(&pool, 10, Turn::led_by(Tier::StarLiked)).await?;
    let rest = claimed.split_off(3);

    assert_eq!(claimed, ["4", "2", "3"]);
    assert_eq!(sorted(rest), ["1", "5"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_rest_turn_keeps_the_old_order(pool: PgPool) -> anyhow::Result<()> {
    seed_unlinked_tracks(&pool).await?;

    let claimed = claimed(&pool, 3, Turn::led_by(Tier::Rest)).await?;

    assert_eq!(sorted(claimed), ["1", "2", "4"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_claimed_track_is_not_claimed_again_while_its_lease_holds(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_unlinked_tracks(&pool).await?;
    let star_first = Turn::led_by(Tier::StarLiked);

    assert_eq!(claimed(&pool, 2, star_first).await?, ["4", "2"]);
    assert_eq!(claimed(&pool, 2, star_first).await?, ["3", "1"]);
    assert_eq!(claimed(&pool, 2, star_first).await?, ["5"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_star_track_the_refresh_found_linked_waits_with_the_rest(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_unlinked_tracks(&pool).await?;
    sqlx::query("UPDATE star_track_tiers SET enrich_due = false WHERE sc_track_id = '4'")
        .execute(&pool)
        .await?;

    assert_eq!(
        claimed(&pool, 1, Turn::led_by(Tier::StarLiked)).await?,
        ["2"]
    );
    Ok(())
}

async fn seed_exhausted_track(
    pool: &PgPool,
    sc_track_id: &str,
    attempts: i16,
    locked_ago: &str,
) -> anyhow::Result<()> {
    sqlx::query(&format!(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             index_priority, enrich_attempts, enrich_locked_at, enrich_next_run_at
         ) VALUES (
             '{sc_track_id}', 'soundcloud:tracks:{sc_track_id}', 'Lost', 'lost', 1000,
             1, {attempts}, now() - interval '{locked_ago}', now() - interval '10 days'
         )"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn state_of(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<(String, Option<String>)> {
    Ok(
        sqlx::query_as("SELECT enrich_state, enrich_error FROM tracks WHERE sc_track_id = $1")
            .bind(sc_track_id)
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_walk_retires_an_exhausted_track_and_does_not_return_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_exhausted_track(&pool, "9", 8, "1 hour").await?;
    seed_unlinked_tracks(&pool).await?;

    let claimed = claimed(&pool, 10, Turn::led_by(Tier::Rest)).await?;

    assert_eq!(sorted(claimed), ["1", "2", "3", "4", "5"]);
    assert_eq!(
        state_of(&pool, "9").await?,
        (
            "dead".to_owned(),
            Some("lease expired after final attempt".to_owned())
        )
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_star_walk_retires_an_exhausted_track_and_does_not_return_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_exhausted_track(&pool, "9", 8, "1 hour").await?;
    sqlx::query("INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('9', 1)")
        .execute(&pool)
        .await?;

    assert!(
        claimed(&pool, 10, Turn::led_by(Tier::StarLiked))
            .await?
            .is_empty()
    );
    assert_eq!(state_of(&pool, "9").await?.0, "dead");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_below_the_cap_is_claimed_and_one_inside_its_lease_is_left_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_exhausted_track(&pool, "7", 7, "1 hour").await?;
    seed_exhausted_track(&pool, "8", 8, "10 seconds").await?;

    assert_eq!(claimed(&pool, 10, Turn::led_by(Tier::Rest)).await?, ["7"]);
    assert_eq!(state_of(&pool, "8").await?.0, "pending");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_repair_migration_resets_lost_attempts_and_leaves_live_work(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             enrich_state, enrich_attempts, enrich_locked_at, enrich_error
         ) VALUES
             ('1', 'soundcloud:tracks:1', 'A', 'a', 1000, 'pending', 5, now() - interval '3 months', NULL),
             ('2', 'soundcloud:tracks:2', 'B', 'b', 1000, 'failed', 5, now() - interval '3 months', 'pool timed out'),
             ('3', 'soundcloud:tracks:3', 'C', 'c', 1000, 'pending', 3, now() - interval '3 months', NULL),
             ('4', 'soundcloud:tracks:4', 'D', 'd', 1000, 'pending', 5, now(), NULL)",
    )
    .execute(&pool)
    .await?;
    let migration = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../api/migrations/0310_tracks_enrich_exhausted_reset.sql"
    ))?;
    sqlx::raw_sql(migration.trim_start_matches("-- no-transaction\n"))
        .execute(&pool)
        .await?;

    let rows: Vec<(String, String, i16, bool, Option<String>)> = sqlx::query_as(
        "SELECT sc_track_id, enrich_state, enrich_attempts, enrich_locked_at IS NULL, enrich_error
         FROM tracks ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        [
            ("1".into(), "pending".into(), 0, true, None),
            ("2".into(), "pending".into(), 0, true, None),
            ("3".into(), "pending".into(), 3, false, None),
            ("4".into(), "pending".into(), 5, false, None),
        ]
    );
    Ok(())
}
