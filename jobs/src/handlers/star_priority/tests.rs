use std::collections::BTreeMap;

use sqlx::PgPool;

use super::{StarTiersHandler, Tier, mark_liked_tracks, of_tracks};

async fn seed_star_library(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO subscriptions (user_urn, exp_date) VALUES
             ('7', extract(epoch FROM now())::bigint + 3600),
             ('8', extract(epoch FROM now())::bigint - 3600);
         INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES
             ('7', '101'),
             ('soundcloud:users:7', '102'),
             ('8', '103');
         INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ('7', '109', false);
         INSERT INTO listening_history (soundcloud_user_id, sc_track_id, title, artist_name, duration, played_at)
         VALUES ('7', '104', 'Recent', 'Artist', 1000, (now() AT TIME ZONE 'UTC') - interval '1 day'),
                ('7', '105', 'Old', 'Artist', 1000, (now() AT TIME ZONE 'UTC') - interval '90 days'),
                ('7', '101', 'Liked', 'Artist', 1000, (now() AT TIME ZONE 'UTC'));
         INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id)
         VALUES ('soundcloud:playlists:1', '1', 'Mine', 'mine', '7');
         INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ('7', 'soundcloud:playlists:1');
         INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ('soundcloud:playlists:1', 0, '106');
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('103', 1), ('106', 1);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn tiers(pool: &PgPool) -> anyhow::Result<BTreeMap<String, i16>> {
    let rows: Vec<(String, i16)> = sqlx::query_as("SELECT sc_track_id, tier FROM star_track_tiers")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

fn expected(pairs: &[(&str, i16)]) -> BTreeMap<String, i16> {
    pairs
        .iter()
        .map(|(id, tier)| ((*id).to_owned(), *tier))
        .collect()
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_refresh_follows_the_current_star_users_only(pool: PgPool) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;

    StarTiersHandler::new(pool.clone()).refresh().await?;

    assert_eq!(
        tiers(&pool).await?,
        expected(&[("101", 1), ("102", 1), ("104", 3), ("106", 3)])
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_refresh_marks_which_pipelines_still_wait_for_each_star_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, storage_state, s3_verified_at
         ) VALUES
             ('101', 'soundcloud:tracks:101', 'Stored', 'stored', 1000, 'ok', now()),
             ('102', 'soundcloud:tracks:102', 'Indexed', 'indexed', 1000, 'ok', now());
         UPDATE tracks SET index_state = 'ok' WHERE sc_track_id = '102';
         UPDATE lyrics_lookup_state SET next_run_at = now() + interval '30 days'
         WHERE sc_track_id = '102';
         INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('102', 'a lyrics text long enough to be embedded later', 'genius');",
    )
    .execute(&pool)
    .await?;

    StarTiersHandler::new(pool.clone()).refresh().await?;

    let due: Vec<(String, bool, bool, bool, bool)> = sqlx::query_as(
        "SELECT sc_track_id, audio_due, lookup_due, embed_due, align_due
         FROM star_track_tiers
         WHERE sc_track_id IN ('101', '102', '104')
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        due,
        [
            ("101".to_owned(), true, true, false, false),
            ("102".to_owned(), false, false, true, false),
            ("104".to_owned(), false, false, false, false),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_lapsed_star_user_drops_out_at_the_next_refresh(pool: PgPool) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    let handler = StarTiersHandler::new(pool.clone());
    handler.refresh().await?;

    sqlx::query("UPDATE subscriptions SET exp_date = 0 WHERE user_urn = '7'")
        .execute(&pool)
        .await?;
    handler.refresh().await?;

    assert!(tiers(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_star_like_is_marked_at_once_and_other_likes_are_not(pool: PgPool) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    let mut connection = pool.acquire().await?;

    mark_liked_tracks(&mut connection, "7", &["106".to_owned(), "201".to_owned()]).await?;
    mark_liked_tracks(&mut connection, "8", &["202".to_owned()]).await?;

    assert_eq!(
        tiers(&pool).await?,
        expected(&[("103", 1), ("106", 1), ("201", 1)])
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_track_tier_puts_star_likes_before_likes_before_star_listening(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, index_priority)
         VALUES ('1', 'soundcloud:tracks:1', 'A', 'a', 1000, 5),
                ('2', 'soundcloud:tracks:2', 'B', 'b', 1000, 1),
                ('3', 'soundcloud:tracks:3', 'C', 'c', 1000, 1),
                ('4', 'soundcloud:tracks:4', 'D', 'd', 1000, 5),
                ('5', 'soundcloud:tracks:5', 'Wave pick', 'wave pick', 1000, 0);
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('1', 1), ('3', 3), ('4', 3);",
    )
    .execute(&pool)
    .await?;
    let ids: Vec<String> = ["1", "2", "3", "4", "5", "6"].map(str::to_owned).to_vec();

    let found = of_tracks(&pool, &ids).await?;

    assert_eq!(found.get("1"), Some(&Tier::StarLiked));
    assert_eq!(found.get("2"), Some(&Tier::Liked));
    assert_eq!(found.get("3"), Some(&Tier::Liked));
    assert_eq!(found.get("5"), Some(&Tier::Rest));
    assert_eq!(found.get("4"), Some(&Tier::StarOther));
    assert_eq!(found.get("6"), None);
    Ok(())
}
