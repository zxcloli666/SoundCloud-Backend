use sqlx::PgPool;
use uuid::Uuid;

use super::DiscoverHandler;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             merged_into uuid,
             crawl_dead boolean NOT NULL DEFAULT false,
             genius_artist_id text,
             genius_crawled_at timestamptz,
             genius_next_run_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE artist_interest (
             artist_id uuid PRIMARY KEY REFERENCES artists (id) ON DELETE CASCADE,
             score real NOT NULL CHECK (score > 0)
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL UNIQUE
         );
         CREATE TABLE track_artists (
             track_id uuid NOT NULL,
             artist_id uuid NOT NULL
         );
         CREATE TABLE user_events (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             sc_track_id text NOT NULL,
             created_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_artist(pool: &PgPool, id: u128) -> anyhow::Result<Uuid> {
    let artist_id = Uuid::from_u128(id);
    sqlx::query(
        "INSERT INTO artists (id, genius_artist_id, genius_next_run_at)
         VALUES ($1, 'genius-' || $1::text, now() + interval '1 day')",
    )
    .bind(artist_id)
    .execute(pool)
    .await?;
    Ok(artist_id)
}

async fn insert_played_artist(pool: &PgPool, id: u128, plays: i32) -> anyhow::Result<Uuid> {
    let artist_id = insert_artist(pool, id).await?;
    let track_id = Uuid::from_u128(id + 1_000_000);
    sqlx::query("INSERT INTO tracks (id, sc_track_id) VALUES ($1, $1::text)")
        .bind(track_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO track_artists (track_id, artist_id) VALUES ($1, $2)")
        .bind(track_id)
        .bind(artist_id)
        .execute(pool)
        .await?;
    play(pool, track_id, plays).await?;
    Ok(artist_id)
}

async fn play(pool: &PgPool, track_id: Uuid, plays: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_events (sc_track_id)
         SELECT $1::text FROM generate_series(1, $2)",
    )
    .bind(track_id)
    .bind(plays)
    .execute(pool)
    .await?;
    Ok(())
}

async fn store_interest(pool: &PgPool, artist_id: Uuid, score: f32) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO artist_interest (artist_id, score) VALUES ($1, $2)")
        .bind(artist_id)
        .bind(score)
        .execute(pool)
        .await?;
    Ok(())
}

async fn interest(pool: &PgPool, artist_id: Uuid) -> anyhow::Result<Option<f32>> {
    Ok(
        sqlx::query_scalar("SELECT score FROM artist_interest WHERE artist_id = $1")
            .bind(artist_id)
            .fetch_optional(pool)
            .await?,
    )
}

async fn surfaced(pool: &PgPool, artist_id: Uuid) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT genius_next_run_at <= now() FROM artists WHERE id = $1")
            .bind(artist_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn row_version(pool: &PgPool, table: &str, column: &str, id: Uuid) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT xmin::text FROM {table} WHERE {column} = $1"
    ))
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn shard_of(pool: &PgPool, artist_id: Uuid, shards: i64) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT ((hashtextextended($1::text, 0) % $2::bigint) + $2::bigint) % $2::bigint",
    )
    .bind(artist_id)
    .bind(shards)
    .fetch_one(pool)
    .await?)
}

async fn recompute(pool: &PgPool, shards: i64, shard: i64) -> anyhow::Result<(i64, i64)> {
    let counts = sqlx::query_file!("queries/discover/interest/recompute.sql", shards, shard)
        .fetch_one(pool)
        .await?;
    Ok((counts.scored, counts.expired))
}

#[sqlx::test(migrations = false)]
async fn recent_activity_scores_and_surfaces_artist(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_played_artist(&pool, 1, 2).await?;

    DiscoverHandler::new(pool.clone(), false, true, 1, 1)
        .recompute_interest()
        .await?;

    assert_eq!(interest(&pool, artist_id).await?, Some(2.0));
    assert!(surfaced(&pool, artist_id).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_changed_score_is_written_over_the_stored_one(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_played_artist(&pool, 1, 3).await?;
    store_interest(&pool, artist_id, 5.0).await?;

    assert_eq!(recompute(&pool, 1, 0).await?, (1, 0));

    assert_eq!(interest(&pool, artist_id).await?, Some(3.0));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn activity_outside_the_window_expires_the_score(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_artist(&pool, 1).await?;
    let track_id = Uuid::from_u128(2);
    sqlx::query("INSERT INTO tracks (id, sc_track_id) VALUES ($1, 'track-1')")
        .bind(track_id)
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO track_artists (track_id, artist_id) VALUES ($1, $2)")
        .bind(track_id)
        .bind(artist_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO user_events (sc_track_id, created_at)
         VALUES ('track-1', now() - interval '31 days')",
    )
    .execute(&pool)
    .await?;
    store_interest(&pool, artist_id, 7.0).await?;

    DiscoverHandler::new(pool.clone(), false, true, 1, 1)
        .recompute_interest()
        .await?;

    assert_eq!(interest(&pool, artist_id).await?, None);
    assert!(!surfaced(&pool, artist_id).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unchanged_score_keeps_its_row_version(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_played_artist(&pool, 1, 2).await?;
    store_interest(&pool, artist_id, 2.0).await?;
    let before = row_version(&pool, "artist_interest", "artist_id", artist_id).await?;

    assert_eq!(recompute(&pool, 1, 0).await?, (0, 0));

    assert_eq!(
        row_version(&pool, "artist_interest", "artist_id", artist_id).await?,
        before
    );
    assert_eq!(interest(&pool, artist_id).await?, Some(2.0));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn one_shard_scores_and_expires_only_its_own_artists(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let shards = 8;
    let mut played = Vec::new();
    let mut stale = Vec::new();
    for index in 0..32 {
        played.push(insert_played_artist(&pool, 100 + index, 1).await?);
        let artist_id = insert_artist(&pool, 200 + index).await?;
        store_interest(&pool, artist_id, 9.0).await?;
        stale.push(artist_id);
    }
    let shard = shard_of(&pool, played[0], shards).await?;

    let (scored, expired) = recompute(&pool, shards, shard).await?;

    let mut in_shard = (0, 0);
    for artist_id in &played {
        let own = shard_of(&pool, *artist_id, shards).await? == shard;
        in_shard.0 += i64::from(own);
        let expected = own.then_some(1.0);
        assert_eq!(interest(&pool, *artist_id).await?, expected);
    }
    for artist_id in &stale {
        let own = shard_of(&pool, *artist_id, shards).await? == shard;
        in_shard.1 += i64::from(own);
        let expected = (!own).then_some(9.0);
        assert_eq!(interest(&pool, *artist_id).await?, expected);
    }
    assert!(in_shard.0 > 0 && in_shard.0 < 32);
    assert!(in_shard.1 > 0 && in_shard.1 < 32);
    assert_eq!((scored, expired), in_shard);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn interest_waits_while_another_artist_update_owns_the_lock(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let mut transaction = pool.begin().await?;
    let acquired = sqlx::query_file_scalar!("queries/discover/try_artist_write_lock.sql")
        .fetch_one(&mut *transaction)
        .await?;
    assert!(acquired);

    let result = DiscoverHandler::new(pool, false, true, 1, 1)
        .recompute_interest()
        .await;

    assert!(result.is_err());
    transaction.rollback().await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn disabled_interest_leaves_existing_scores_untouched(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_artist(&pool, 1).await?;
    store_interest(&pool, artist_id, 7.0).await?;

    DiscoverHandler::new(pool.clone(), false, false, 1, 1)
        .recompute_interest()
        .await?;

    assert_eq!(interest(&pool, artist_id).await?, Some(7.0));
    Ok(())
}

async fn insert_catalog_artist(pool: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source, genius_artist_id, genius_crawled_at)
         VALUES ($1, $1, 'soundcloud', $1, now())
         RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await?)
}

async fn insert_catalog_track(
    pool: &PgPool,
    sc_track_id: &str,
    artist_id: Uuid,
    plays: i32,
) -> anyhow::Result<()> {
    let track_id: Uuid = sqlx::query_scalar(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ($1, 'soundcloud:tracks:' || $1, $1, $1, 180000)
         RETURNING id",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source)
         VALUES ($1, $2, 'primary', 'heuristic')",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight)
         SELECT 'listener-' || n, $1, 'full_play', 1
         FROM generate_series(1, $2) AS n",
    )
    .bind(sc_track_id)
    .bind(plays)
    .execute(pool)
    .await?;
    Ok(())
}

async fn artist_versions(pool: &PgPool, artists: &[Uuid]) -> anyhow::Result<Vec<String>> {
    let mut versions = Vec::new();
    for artist_id in artists {
        versions.push(row_version(pool, "artists", "id", *artist_id).await?);
    }
    Ok(versions)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_recompute_rewrites_no_artists_row(pool: PgPool) -> anyhow::Result<()> {
    let new = insert_catalog_artist(&pool, "new").await?;
    let changed = insert_catalog_artist(&pool, "changed").await?;
    let expired = insert_catalog_artist(&pool, "expired").await?;
    insert_catalog_track(&pool, "t1", new, 1).await?;
    insert_catalog_track(&pool, "t2", changed, 2).await?;
    store_interest(&pool, changed, 9.0).await?;
    store_interest(&pool, expired, 4.0).await?;
    let artists = [new, changed, expired];
    let before = artist_versions(&pool, &artists).await?;

    DiscoverHandler::new(pool.clone(), false, true, 1, 1)
        .recompute_interest()
        .await?;

    assert_eq!(interest(&pool, new).await?, Some(1.0));
    assert_eq!(interest(&pool, changed).await?, Some(2.0));
    assert_eq!(interest(&pool, expired).await?, None);
    assert_eq!(artist_versions(&pool, &artists).await?, before);
    Ok(())
}
