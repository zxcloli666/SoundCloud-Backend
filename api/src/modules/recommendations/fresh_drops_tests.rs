use sqlx::PgPool;

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    let statements = [
        "INSERT INTO artists (id, name, normalized_name, source)
         SELECT lpad(to_hex(n), 32, '0')::uuid, 'a' || n, 'a' || n, 'test'
         FROM generate_series(1, 3) AS n",
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing,
                             storage_state, index_state, needs_duration_resolve, sc_created_at)
         SELECT sc, 'urn:' || sc, 'T', 't', 1000, 'public', 'ok', 'indexed', unready,
                now() - age_days * interval '1 day'
         FROM (VALUES ('liked1', 200, false), ('liked2', 210, false), ('liked3', 220, false),
                      ('liked4', 230, false), ('liked5', 240, false),
                      ('new', 5, false), ('shared', 8, false), ('mid', 40, false),
                      ('old', 120, false), ('unready', 10, true), ('other', 1, false))
              AS seeded(sc, age_days, unready)",
        "INSERT INTO track_artists (track_id, artist_id, role, source)
         SELECT track.id, lpad(to_hex(credit.artist), 32, '0')::uuid, 'primary', 'test'
         FROM (VALUES ('liked1', 1), ('liked2', 1), ('liked3', 3), ('liked4', 3), ('liked5', 2),
                      ('new', 1), ('shared', 1), ('shared', 3), ('mid', 1), ('old', 1),
                      ('unready', 1), ('other', 2))
              AS credit(sc, artist)
         JOIN tracks AS track ON track.sc_track_id = credit.sc",
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
         SELECT 'u1', sc, true, now() - interval '1 day'
         FROM unnest(ARRAY['liked1', 'liked2', 'liked3', 'liked4', 'liked5']) AS sc",
    ];
    for statement in statements {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

async fn fresh(pool: &PgPool, exclude: &[&str], limit: i64) -> anyhow::Result<Vec<String>> {
    let users = vec!["u1".to_owned()];
    let exclude: Vec<String> = exclude.iter().map(|id| (*id).to_owned()).collect();
    Ok(sqlx::query_file_scalar!(
        "queries/recommendations/home_wave/fresh_drops.sql",
        &users,
        &exclude,
        limit
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn fresh_drops_are_the_newest_uploads_of_the_liked_artists_within_ninety_days(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    assert_eq!(fresh(&pool, &[], 24).await?, ["new", "shared", "mid"]);
    assert_eq!(fresh(&pool, &["shared"], 24).await?, ["new", "mid"]);
    assert_eq!(fresh(&pool, &[], 1).await?, ["new"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_track_uploaded_long_ago_and_synced_today_is_not_fresh(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    sqlx::query(
        "INSERT INTO track_sync_stamps (track_id, sc_synced_at)
         SELECT id, now() FROM tracks
         ON CONFLICT (track_id) DO UPDATE SET sc_synced_at = EXCLUDED.sc_synced_at",
    )
    .execute(&pool)
    .await?;
    assert!(!fresh(&pool, &[], 24).await?.contains(&"old".to_owned()));
    Ok(())
}
