use sqlx::PgPool;
use uuid::Uuid;

use super::paced::id_ranges;
use super::{DiscoverHandler, Group};

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             merged_into uuid,
             popularity_score real NOT NULL DEFAULT 0,
             tags text[] NOT NULL DEFAULT '{}'
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL UNIQUE,
             primary_artist_id uuid,
             genre text
         );
         CREATE TABLE sc_track_counters (
             sc_track_id text PRIMARY KEY,
             play_count bigint
         );
         CREATE TABLE user_events (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             sc_track_id text NOT NULL,
             event_type text NOT NULL
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

async fn add_artist(pool: &PgPool, id: u128, merged_into: Option<u128>) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO artists (id, merged_into) VALUES ($1, $2)")
        .bind(Uuid::from_u128(id))
        .bind(merged_into.map(Uuid::from_u128))
        .execute(pool)
        .await?;
    Ok(())
}

async fn add_track(
    pool: &PgPool,
    sc_track_id: &str,
    artist: u128,
    genre: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (id, sc_track_id, primary_artist_id, genre)
         VALUES (gen_random_uuid(), $1, $2, $3)",
    )
    .bind(sc_track_id)
    .bind(Uuid::from_u128(artist))
    .bind(genre)
    .execute(pool)
    .await?;
    Ok(())
}

async fn popularity_of(pool: &PgPool, id: u128) -> anyhow::Result<f32> {
    Ok(
        sqlx::query_scalar("SELECT popularity_score FROM artists WHERE id = $1")
            .bind(Uuid::from_u128(id))
            .fetch_one(pool)
            .await?,
    )
}

fn handler(pool: &PgPool) -> DiscoverHandler {
    DiscoverHandler::new(pool.clone(), false, true, 1, 1)
}

#[test]
fn the_id_ranges_tile_the_whole_uuid_space() {
    let ranges: Vec<_> = id_ranges().collect();
    assert_eq!(ranges.len(), 256);
    assert_eq!(ranges[0].first, Uuid::nil());
    assert_eq!(ranges[255].last, Uuid::max());
    for pair in ranges.windows(2) {
        assert_eq!(pair[0].last.as_u128() + 1, pair[1].first.as_u128());
    }
}

#[sqlx::test(migrations = false)]
async fn popularity_follows_the_play_totals_against_the_busiest_artist(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let busiest = 1u128 << 127;
    let quiet = 5u128;
    let silent = 6u128;
    let merged = 7u128;
    for id in [busiest, quiet, silent] {
        add_artist(&pool, id, None).await?;
    }
    add_artist(&pool, merged, Some(busiest)).await?;
    add_track(&pool, "busy", busiest, None).await?;
    add_track(&pool, "quiet", quiet, None).await?;
    add_track(&pool, "merged", merged, None).await?;
    sqlx::raw_sql(
        "INSERT INTO sc_track_counters VALUES ('busy', 100), ('quiet', 9), ('merged', 50);
         INSERT INTO user_events (sc_track_id, event_type) VALUES
             ('busy', 'full_play'), ('busy', 'full_play'), ('busy', 'skip');",
    )
    .execute(&pool)
    .await?;

    handler(&pool)
        .refresh_aggregates(Group::ArtistPopularity)
        .await?;

    let expected_quiet = ((10f64.ln() / 20_101f64.ln()) * 10_000.0).round() / 10_000.0;
    assert_eq!(popularity_of(&pool, busiest).await?, 1.0);
    assert_eq!(popularity_of(&pool, quiet).await?, expected_quiet as f32);
    assert_eq!(popularity_of(&pool, silent).await?, 0.0);
    assert_eq!(popularity_of(&pool, merged).await?, 0.0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_second_popularity_refresh_without_new_plays_rewrites_no_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    add_artist(&pool, 1, None).await?;
    add_artist(&pool, 1 << 127, None).await?;
    add_track(&pool, "a", 1, None).await?;
    add_track(&pool, "b", 1 << 127, None).await?;
    sqlx::raw_sql("INSERT INTO sc_track_counters VALUES ('a', 40), ('b', 400)")
        .execute(&pool)
        .await?;
    handler(&pool)
        .refresh_aggregates(Group::ArtistPopularity)
        .await?;
    let versions = |pool: PgPool| async move {
        sqlx::query_scalar::<_, String>("SELECT xmin::text FROM artists ORDER BY id")
            .fetch_all(&pool)
            .await
    };
    let before = versions(pool.clone()).await?;

    handler(&pool)
        .refresh_aggregates(Group::ArtistPopularity)
        .await?;

    assert_eq!(versions(pool.clone()).await?, before);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_totals_do_not_outlive_a_refresh_on_the_connection(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    add_artist(&pool, 1, None).await?;
    let single = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*pool.connect_options()).clone())
        .await?;

    handler(&single)
        .refresh_aggregates(Group::ArtistPopularity)
        .await?;

    let leftover: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('pg_temp.artist_play_totals')::text")
            .fetch_one(&single)
            .await?;
    assert_eq!(leftover, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn tags_are_the_top_three_genres_by_track_count_then_name(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = 3u128 << 120;
    let bare = 9u128;
    add_artist(&pool, artist, None).await?;
    add_artist(&pool, bare, None).await?;
    sqlx::query("UPDATE artists SET tags = ARRAY['stale'] WHERE id = $1")
        .bind(Uuid::from_u128(bare))
        .execute(&pool)
        .await?;
    let genres = [
        "Rock", " rock ", "ROCK", "jazz", "Jazz", "folk", "blues", "ambient", "  ", "",
    ];
    for (index, genre) in genres.iter().enumerate() {
        add_track(&pool, &format!("t{index}"), artist, Some(genre)).await?;
    }
    add_track(&pool, "no-genre", artist, None).await?;

    handler(&pool).refresh_aggregates(Group::ArtistTags).await?;

    let tags: Vec<String> = sqlx::query_scalar("SELECT unnest(tags) FROM artists WHERE id = $1")
        .bind(Uuid::from_u128(artist))
        .fetch_all(&pool)
        .await?;
    assert_eq!(tags, vec!["rock", "jazz", "ambient"]);
    let cleared: Vec<String> = sqlx::query_scalar("SELECT unnest(tags) FROM artists WHERE id = $1")
        .bind(Uuid::from_u128(bare))
        .fetch_all(&pool)
        .await?;
    assert!(cleared.is_empty());
    Ok(())
}
