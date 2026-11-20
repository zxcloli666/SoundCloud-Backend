use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_of_the_artist_with_the_same_title_matches(pool: PgPool) -> anyhow::Result<()> {
    let artist = new_artist(&pool, "a1").await?;
    let other = new_artist(&pool, "a2").await?;
    let own = track(&pool, "s1", Some(artist), "Song Name", "song name").await?;
    track(&pool, "s2", Some(other), "Song Name", "song name").await?;

    let found = best(&pool, artist, "Song Name").await?;

    assert_eq!(found, Some(own));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_the_artist_is_only_credited_on_matches(pool: PgPool) -> anyhow::Result<()> {
    let artist = new_artist(&pool, "a1").await?;
    let owner = new_artist(&pool, "a2").await?;
    let shared = track(&pool, "s1", Some(owner), "Song Name", "song name").await?;
    credit(&pool, shared, owner).await?;
    credit(&pool, shared, artist).await?;

    assert_eq!(best(&pool, artist, "Song Name").await?, Some(shared));
    assert_eq!(best(&pool, owner, "Song Name").await?, Some(shared));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_superseded_track_and_another_artists_track_do_not_match(
    pool: PgPool,
) -> anyhow::Result<()> {
    let artist = new_artist(&pool, "a1").await?;
    let other = new_artist(&pool, "a2").await?;
    let kept = track(&pool, "s1", Some(other), "Song Name", "song name").await?;
    let old = track(&pool, "s2", Some(artist), "Song Name", "song name").await?;
    sqlx::query("UPDATE tracks SET superseded_by = $2 WHERE id = $1")
        .bind(old)
        .bind(kept)
        .execute(&pool)
        .await?;

    assert_eq!(best(&pool, artist, "Song Name").await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_exact_title_is_kept_when_the_artist_has_more_prefix_matches_than_the_limit(
    pool: PgPool,
) -> anyhow::Result<()> {
    let artist = new_artist(&pool, "a1").await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, primary_artist_id, duration_ms, sharing)
         SELECT 'p' || n, 'soundcloud:tracks:p' || n, 'Song ' || n || ' Remix', 'song ' || n || ' remix', $1, 180000, 'public'
         FROM generate_series(1, 300) AS n",
    )
    .bind(artist)
    .execute(&pool)
    .await?;
    let exact = track(&pool, "zz", Some(artist), "Song Name", "song name").await?;

    assert_eq!(best(&pool, artist, "Song Name").await?, Some(exact));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_work_key_match_is_found_without_a_matching_title(pool: PgPool) -> anyhow::Result<()> {
    let artist = new_artist(&pool, "a1").await?;
    let keyed = track(&pool, "s1", Some(artist), "Song Name", "unrelated").await?;
    let work_key = catalog_normalize::title_forms("Song Name").work_key;
    sqlx::query("UPDATE track_work_keys SET work_key = $2 WHERE track_id = $1")
        .bind(keyed)
        .bind(&work_key)
        .execute(&pool)
        .await?;

    assert_eq!(best(&pool, artist, "Song Name").await?, Some(keyed));
    Ok(())
}

async fn best(pool: &PgPool, artist: Uuid, title: &str) -> anyhow::Result<Option<Uuid>> {
    Ok(
        catalog_match::best_indexed_for_artist_title(pool, artist, title)
            .await?
            .map(|found| found.track_id),
    )
}

async fn new_artist(pool: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source)
         VALUES ($1, $1, 'sc')
         RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await?)
}

async fn track(
    pool: &PgPool,
    sc_track_id: &str,
    primary_artist_id: Option<Uuid>,
    title: &str,
    title_normalized: &str,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, primary_artist_id, duration_ms, sharing
         )
         VALUES ($1, 'soundcloud:tracks:' || $1, $2, $3, $4, 180000, 'public')
         RETURNING id",
    )
    .bind(sc_track_id)
    .bind(title)
    .bind(title_normalized)
    .bind(primary_artist_id)
    .fetch_one(pool)
    .await?)
}

async fn credit(pool: &PgPool, track: Uuid, artist: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source)
         VALUES ($1, $2, 'primary', 'sc')",
    )
    .bind(track)
    .bind(artist)
    .execute(pool)
    .await?;
    Ok(())
}
