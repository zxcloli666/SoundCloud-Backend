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

#[sqlx::test(migrations = "../api/migrations")]
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

#[sqlx::test(migrations = "../api/migrations")]
async fn a_rest_turn_keeps_the_old_order(pool: PgPool) -> anyhow::Result<()> {
    seed_unlinked_tracks(&pool).await?;

    let claimed = claimed(&pool, 3, Turn::led_by(Tier::Rest)).await?;

    assert_eq!(sorted(claimed), ["1", "2", "4"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
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

#[sqlx::test(migrations = "../api/migrations")]
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
