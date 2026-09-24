use sqlx::PgPool;
use uuid::Uuid;

use super::DiscoverHandler;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             interest_score real NOT NULL DEFAULT 0,
             monthly_listeners bigint NOT NULL DEFAULT 0,
             trending_score real NOT NULL DEFAULT 0,
             merged_into uuid,
             crawl_dead boolean NOT NULL DEFAULT false,
             genius_artist_id text,
             genius_crawled_at timestamptz,
             genius_next_run_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL UNIQUE,
             primary_artist_id uuid
         );
         CREATE TABLE track_artists (
             track_id uuid NOT NULL,
             artist_id uuid NOT NULL
         );
         CREATE TABLE user_events (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL,
             sc_user_id text NOT NULL DEFAULT 'listener-1',
             event_type text NOT NULL DEFAULT 'full_play',
             created_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_artist_track(pool: &PgPool, interest_score: f32) -> anyhow::Result<Uuid> {
    let artist_id = Uuid::from_u128(1);
    let track_id = Uuid::from_u128(2);
    sqlx::query(
        "INSERT INTO artists (
             id, interest_score, genius_artist_id, genius_next_run_at
         ) VALUES ($1, $2, 'genius-1', now() + interval '1 day')",
    )
    .bind(artist_id)
    .bind(interest_score)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO tracks (id, sc_track_id) VALUES ($1, 'track-1')")
        .bind(track_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO track_artists (track_id, artist_id) VALUES ($1, $2)")
        .bind(track_id)
        .bind(artist_id)
        .execute(pool)
        .await?;
    Ok(artist_id)
}

#[sqlx::test(migrations = false)]
async fn recent_activity_scores_and_surfaces_artist(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_artist_track(&pool, 0.0).await?;
    sqlx::query(
        "INSERT INTO user_events (id, sc_track_id)
         VALUES ($1, 'track-1'), ($2, 'track-1')",
    )
    .bind(Uuid::from_u128(3))
    .bind(Uuid::from_u128(4))
    .execute(&pool)
    .await?;

    DiscoverHandler::new(pool.clone(), false, true, 1, 1)
        .recompute_interest()
        .await?;

    let artist: (f32, bool) = sqlx::query_as(
        "SELECT interest_score, genius_next_run_at <= now()
         FROM artists WHERE id = $1",
    )
    .bind(artist_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(artist, (2.0, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn activity_outside_the_window_clears_stale_score(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = insert_artist_track(&pool, 7.0).await?;
    sqlx::query(
        "INSERT INTO user_events (id, sc_track_id, created_at)
         VALUES ($1, 'track-1', now() - interval '31 days')",
    )
    .bind(Uuid::from_u128(3))
    .execute(&pool)
    .await?;

    DiscoverHandler::new(pool.clone(), false, true, 1, 1)
        .recompute_interest()
        .await?;

    let artist: (f32, bool) = sqlx::query_as(
        "SELECT interest_score, genius_next_run_at > now()
         FROM artists WHERE id = $1",
    )
    .bind(artist_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(artist, (0.0, true));
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
    let artist_id = insert_artist_track(&pool, 7.0).await?;

    DiscoverHandler::new(pool.clone(), false, false, 1, 1)
        .recompute_interest()
        .await?;

    let score: f32 = sqlx::query_scalar("SELECT interest_score FROM artists WHERE id = $1")
        .bind(artist_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(score, 7.0);
    Ok(())
}

async fn install_tag_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             merged_into uuid,
             tags text[] NOT NULL DEFAULT '{}'::text[]
         );
         CREATE TABLE discover_tag_counts (
             tag text PRIMARY KEY,
             artist_count bigint NOT NULL,
             refreshed_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_tagged_artist(pool: &PgPool, id: u128, tags: &[&str]) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO artists (id, tags) VALUES ($1, $2)")
        .bind(Uuid::from_u128(id))
        .bind(tags.iter().map(|tag| (*tag).to_owned()).collect::<Vec<_>>())
        .execute(pool)
        .await?;
    Ok(())
}

async fn tag_counts(pool: &PgPool) -> anyhow::Result<Vec<(String, i64)>> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT tag, artist_count FROM discover_tag_counts ORDER BY artist_count DESC, tag",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[sqlx::test(migrations = false)]
async fn tag_counts_come_from_live_artists_only(pool: PgPool) -> anyhow::Result<()> {
    install_tag_schema(&pool).await?;
    insert_tagged_artist(&pool, 1, &["house", "techno"]).await?;
    insert_tagged_artist(&pool, 2, &["house"]).await?;
    insert_tagged_artist(&pool, 3, &[]).await?;
    insert_tagged_artist(&pool, 4, &["  "]).await?;
    sqlx::query("INSERT INTO artists (id, merged_into, tags) VALUES ($1, $2, $3)")
        .bind(Uuid::from_u128(5))
        .bind(Uuid::from_u128(1))
        .bind(vec!["house".to_owned(), "trance".to_owned()])
        .execute(&pool)
        .await?;

    sqlx::query_file!("queries/discover/refresh_tag_counts.sql")
        .execute(&pool)
        .await?;

    assert_eq!(
        tag_counts(&pool).await?,
        vec![("house".to_owned(), 2), ("techno".to_owned(), 1)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_tag_that_no_artist_carries_anymore_disappears(pool: PgPool) -> anyhow::Result<()> {
    install_tag_schema(&pool).await?;
    sqlx::query("INSERT INTO discover_tag_counts (tag, artist_count) VALUES ('gone', 99)")
        .execute(&pool)
        .await?;
    insert_tagged_artist(&pool, 1, &["house"]).await?;

    sqlx::query_file!("queries/discover/refresh_tag_counts.sql")
        .execute(&pool)
        .await?;

    assert_eq!(tag_counts(&pool).await?, vec![("house".to_owned(), 1)]);
    Ok(())
}

async fn install_listened_artists(pool: &PgPool, artists: u32) -> anyhow::Result<()> {
    for index in 0..artists {
        let artist_id = Uuid::from_u128(u128::from(index) + 1000);
        let track_id = Uuid::from_u128(u128::from(index) + 2000);
        let sc_track_id = format!("track-{index}");
        sqlx::query("INSERT INTO artists (id) VALUES ($1)")
            .bind(artist_id)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO tracks (id, sc_track_id, primary_artist_id) VALUES ($1, $2, $3)")
            .bind(track_id)
            .bind(&sc_track_id)
            .bind(artist_id)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO user_events (id, sc_track_id, sc_user_id, event_type)
             VALUES ($1, $2, 'listener-1', 'full_play')",
        )
        .bind(Uuid::from_u128(u128::from(index) + 3000))
        .bind(&sc_track_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn listeners_written(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM artists WHERE monthly_listeners > 0")
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrations = false)]
async fn one_pass_of_the_play_refresh_touches_only_its_own_share(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    install_listened_artists(&pool, 64).await?;

    let mut transaction = pool.begin().await?;
    super::Stage::ArtistPlays
        .execute(&mut transaction, super::Shards::at(8, 0))
        .await?;
    transaction.commit().await?;

    let touched = listeners_written(&pool).await?;
    assert!(
        touched > 0,
        "a share that updates nobody would make the whole rotation a no-op"
    );
    assert!(
        touched < 64,
        "the share updated all {touched} artists, so the batch is still unbounded and one \
         transaction still rewrites every artist there is"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_whole_rotation_leaves_no_artist_behind(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    install_listened_artists(&pool, 64).await?;

    for shard in 0..8 {
        let mut transaction = pool.begin().await?;
        super::Stage::ArtistPlays
            .execute(&mut transaction, super::Shards::at(8, shard))
            .await?;
        transaction.commit().await?;
    }

    assert_eq!(
        listeners_written(&pool).await?,
        64,
        "every artist must be reached within one rotation, or their listener count freezes \
         at whatever it was when the sharding landed"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_single_share_is_the_old_unsharded_pass(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    install_listened_artists(&pool, 16).await?;

    let mut transaction = pool.begin().await?;
    super::Stage::ArtistPlays
        .execute(&mut transaction, super::Shards::at(1, 7))
        .await?;
    transaction.commit().await?;

    assert_eq!(
        listeners_written(&pool).await?,
        16,
        "setting the shard count to one must switch the split off entirely"
    );
    Ok(())
}

#[test]
fn the_rotation_visits_every_share_once_per_cycle() {
    let seen: std::collections::BTreeSet<i64> = (0..8)
        .map(|hour| super::Shards::at(8, hour).current)
        .collect();
    assert_eq!(seen.len(), 8, "eight hours must cover eight shares");

    assert_eq!(super::Shards::at(8, 8).current, 0, "the cycle repeats");
    assert_eq!(
        super::Shards::at(8, -1).current,
        7,
        "an hour before the epoch must still name a real share, not a negative one"
    );
    assert_eq!(
        super::Shards::at(0, 5).current,
        0,
        "zero shares is one share"
    );
}
