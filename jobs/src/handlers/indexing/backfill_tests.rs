use sqlx::PgPool;

use super::pick;
use crate::handlers::star_priority::{Tier, Turn};

async fn seed_stored_tracks(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             storage_state, s3_verified_at, index_priority, created_at
         ) VALUES
             ('1', 'soundcloud:tracks:1', 'Oldest', 'oldest', 1000, 'ok', now(), 5, now() - interval '5 days'),
             ('2', 'soundcloud:tracks:2', 'Liked', 'liked', 1000, 'ok', now(), 1, now() - interval '4 days'),
             ('3', 'soundcloud:tracks:3', 'Star other', 'star other', 1000, 'ok', now(), 5, now() - interval '3 days'),
             ('4', 'soundcloud:tracks:4', 'Star liked', 'star liked', 1000, 'ok', now(), 1, now() - interval '2 days'),
             ('5', 'soundcloud:tracks:5', 'Discovery', 'discovery', 1000, 'ok', now(), 5, now() - interval '1 day'),
             ('6', 'soundcloud:tracks:6', 'Wave pick', 'wave pick', 1000, 'ok', now(), 0, now() - interval '6 days');
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('3', 3), ('4', 1);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn picked(pool: &PgPool, room: i64, turn: Turn) -> anyhow::Result<Vec<String>> {
    Ok(pick(pool, room, turn)
        .await?
        .into_iter()
        .map(|candidate| candidate.sc_track_id)
        .collect())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_star_turn_indexes_star_likes_then_likes_then_star_tracks_then_the_rest(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_tracks(&pool).await?;
    let star_first = Turn::led_by(Tier::StarLiked);

    assert_eq!(
        picked(&pool, 10, star_first).await?,
        ["4", "2", "3", "6", "1", "5"]
    );
    assert_eq!(picked(&pool, 2, star_first).await?, ["4", "2"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_wave_pick_waits_with_the_rest_and_not_with_the_likes(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_tracks(&pool).await?;

    assert_eq!(picked(&pool, 1, Turn::led_by(Tier::Liked)).await?, ["2"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn other_star_tracks_lead_their_own_turn(pool: PgPool) -> anyhow::Result<()> {
    seed_stored_tracks(&pool).await?;

    assert_eq!(
        picked(&pool, 2, Turn::led_by(Tier::StarOther)).await?,
        ["3", "4"]
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_rest_turn_keeps_the_old_order(pool: PgPool) -> anyhow::Result<()> {
    seed_stored_tracks(&pool).await?;

    assert_eq!(
        picked(&pool, 10, Turn::led_by(Tier::Rest)).await?,
        ["6", "2", "4", "1", "3", "5"]
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_star_track_the_refresh_found_indexed_is_not_walked_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_tracks(&pool).await?;
    sqlx::query("UPDATE star_track_tiers SET audio_due = false WHERE sc_track_id = '4'")
        .execute(&pool)
        .await?;

    assert_eq!(
        picked(&pool, 1, Turn::led_by(Tier::StarLiked)).await?,
        ["2"]
    );
    Ok(())
}
