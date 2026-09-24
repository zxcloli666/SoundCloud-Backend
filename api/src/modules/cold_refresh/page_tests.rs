use sqlx::PgPool;

use super::read::collection_page_keys;
use super::service::{FOLLOWINGS, LIKED_TRACKS, OWNED_PLAYLISTS, OWNED_TRACKS};

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql("CREATE TABLE tracks (sc_track_id text PRIMARY KEY, sharing text NOT NULL, release_date date, sc_created_at timestamptz, deleted_at timestamptz);
        CREATE TABLE users (urn text PRIMARY KEY);
        CREATE TABLE playlists (urn text PRIMARY KEY, sharing text NOT NULL, deleted_at timestamptz);
        CREATE TABLE user_likes_tracks (user_id text, sc_track_id text, wanted_state boolean, created_at timestamptz);
        CREATE TABLE user_owned_playlists (user_id text, playlist_urn text, created_at timestamptz);
        CREATE TABLE user_owned_tracks (user_id text, sc_track_id text, created_at timestamptz);
        CREATE TABLE user_followings (user_id text, target_user_urn text, wanted_state boolean, created_at timestamptz)")
        .execute(pool).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn public_pages_skip_private_missing_and_unwanted_tracks_before_limit(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql("INSERT INTO tracks (sc_track_id, sharing) VALUES ('5', 'private'), ('4', 'public'), ('3', 'public'), ('2', 'public');
        INSERT INTO user_likes_tracks VALUES ('42', '6', true, now()), ('42', '5', true, now()),
        ('42', '4', false, now()), ('42', '3', true, now()), ('soundcloud:users:42', '3', true, now()),
        ('42', '2', true, now())").execute(&pool).await?;
    assert_eq!(
        collection_page_keys(&pool, &LIKED_TRACKS, "42", 0, 1, true).await?,
        vec!["3", "2"]
    );
    assert_eq!(
        collection_page_keys(&pool, &LIKED_TRACKS, "42", 1, 1, true).await?,
        vec!["2"]
    );
    assert!(
        collection_page_keys(&pool, &LIKED_TRACKS, "42", 2, 1, true)
            .await?
            .is_empty()
    );
    assert_eq!(
        collection_page_keys(&pool, &LIKED_TRACKS, "42", 0, 2, false).await?,
        vec!["5", "3", "2"]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn canonical_unlike_hides_an_older_urn_row_before_pagination(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, sharing) VALUES ('1', 'public'), ('2', 'public');
        INSERT INTO user_likes_tracks VALUES ('42', '1', false, now()),
        ('soundcloud:users:42', '1', true, now() - interval '1 day'), ('42', '2', true, now())",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        collection_page_keys(&pool, &LIKED_TRACKS, "42", 0, 1, true).await?,
        ["2"]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn playlist_pages_apply_visibility_before_limit(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql("INSERT INTO playlists VALUES ('soundcloud:playlists:3', 'private'), ('soundcloud:playlists:2', 'public');
        INSERT INTO user_owned_playlists VALUES ('42', 'soundcloud:playlists:4', now()),
        ('42', 'soundcloud:playlists:3', now()), ('42', 'soundcloud:playlists:2', now())")
        .execute(&pool).await?;
    assert_eq!(
        collection_page_keys(&pool, &OWNED_PLAYLISTS, "42", 0, 1, true).await?,
        vec!["soundcloud:playlists:2"]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn following_pages_skip_missing_profiles_before_limit(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql("INSERT INTO users VALUES ('soundcloud:users:2');
        INSERT INTO user_followings VALUES ('42', 'soundcloud:users:3', true, now()), ('42', 'soundcloud:users:2', true, now())")
        .execute(&pool).await?;
    assert_eq!(
        collection_page_keys(&pool, &FOLLOWINGS, "42", 0, 1, true).await?,
        vec!["soundcloud:users:2"]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn owned_track_pages_preserve_release_order_after_visibility_filter(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks VALUES ('1', 'public', '2026-09-03', now() - interval '1 day'),
        ('2', 'public', '2026-09-01', now()), ('3', 'private', '2026-09-05', now());
        INSERT INTO user_owned_tracks VALUES ('42', '1', now() - interval '1 day'),
        ('42', '2', now()), ('42', '3', now())",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        collection_page_keys(&pool, &OWNED_TRACKS, "42", 0, 1, true).await?,
        vec!["1", "2"]
    );
    assert!(
        collection_page_keys(&pool, &OWNED_TRACKS, "42", i64::MAX, 200, true)
            .await
            .is_err()
    );
    Ok(())
}
