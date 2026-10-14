use sqlx::PgPool;

use super::pick;
use crate::handlers::star_priority::Turn;

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
             ('5', 'soundcloud:tracks:5', 'Discovery', 'discovery', 1000, 'ok', now(), 5, now() - interval '1 day');
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

    assert_eq!(
        picked(&pool, 10, Turn::StarFirst).await?,
        ["4", "2", "3", "1", "5"]
    );
    assert_eq!(picked(&pool, 2, Turn::StarFirst).await?, ["4", "2"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_rest_turn_keeps_the_old_order(pool: PgPool) -> anyhow::Result<()> {
    seed_stored_tracks(&pool).await?;

    assert_eq!(
        picked(&pool, 10, Turn::RestFirst).await?,
        ["2", "4", "1", "3", "5"]
    );
    Ok(())
}
