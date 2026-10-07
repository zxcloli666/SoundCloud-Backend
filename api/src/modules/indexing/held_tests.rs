use serde_json::{Value, json};
use sqlx::PgPool;

fn seen(id: u64, title: &str, user: u64) -> Value {
    json!({
        "kind": "track",
        "id": id,
        "urn": format!("soundcloud:tracks:{id}"),
        "title": title,
        "duration": 200000,
        "full_duration": 200000,
        "access": "playable",
        "policy": "ALLOW",
        "likes_count": 12,
        "user": {
            "kind": "user",
            "id": user,
            "urn": format!("soundcloud:users:{user}"),
            "username": format!("artist {user}"),
        }
    })
}

async fn held(pool: &PgPool, id: &str) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT pipeline_held FROM tracks WHERE sc_track_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

async fn lyrics_state(pool: &PgPool, id: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM lyrics_lookup_state WHERE sc_track_id = $1)",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_seen_track_is_only_stored_and_never_overwrites_a_richer_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, play_count_sc)
         VALUES ('5', 'soundcloud:tracks:5', 'Rich', 'rich', 1000, 99)",
    )
    .execute(&pool)
    .await?;

    let inserted = catalog_ingest::insert_absent_tracks(
        &pool,
        &[
            seen(5, "Poor", 70),
            seen(6, "Fresh", 77),
            seen(6, "Fresh again", 77),
            json!({"urn": "soundcloud:users:8", "title": "not a track"}),
        ],
        catalog_ingest::Observation::UNVERIFIED,
    )
    .await?;
    assert_eq!(inserted.tracks, vec!["6".to_owned()]);
    let mut users = inserted.users.clone();
    users.sort();
    assert_eq!(users, vec!["70".to_owned(), "77".to_owned()]);

    let (title, plays): (String, Option<i64>) =
        sqlx::query_as("SELECT title, play_count_sc FROM tracks WHERE sc_track_id = '5'")
            .fetch_one(&pool)
            .await?;
    assert_eq!((title.as_str(), plays), ("Rich", Some(99)));
    assert!(!held(&pool, "5").await?);

    let (title, metadata): (String, Value) =
        sqlx::query_as("SELECT title, sc_metadata FROM tracks WHERE sc_track_id = '6'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(title, "Fresh");
    assert_eq!(metadata["access"], "playable");
    assert_eq!(metadata["policy"], "ALLOW");
    assert!(held(&pool, "6").await?);
    assert!(
        !lyrics_state(&pool, "6").await?,
        "a search hit must not wake the lyrics pipeline"
    );

    let again = catalog_ingest::insert_absent_tracks(
        &pool,
        &[seen(6, "Changed", 77)],
        catalog_ingest::Observation::UNVERIFIED,
    )
    .await?;
    assert_eq!(again, catalog_ingest::AbsentInserted::default());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn opening_or_a_full_ingest_releases_a_held_track(pool: PgPool) -> anyhow::Result<()> {
    catalog_ingest::insert_absent_tracks(
        &pool,
        &[seen(6, "Fresh", 77), seen(7, "Other", 77)],
        catalog_ingest::Observation::UNVERIFIED,
    )
    .await?;

    let released: Option<i32> =
        sqlx::query_file_scalar!("queries/indexing/service/release_held.sql", "6")
            .fetch_optional(&pool)
            .await?;
    assert_eq!(released, Some(200000));
    assert!(!held(&pool, "6").await?);
    assert!(lyrics_state(&pool, "6").await?);
    let again: Option<i32> =
        sqlx::query_file_scalar!("queries/indexing/service/release_held.sql", "6")
            .fetch_optional(&pool)
            .await?;
    assert_eq!(again, None, "a released track is kicked once");

    let fields = catalog_ingest::ScTrackFields::from_sc(&seen(7, "Other", 77)).unwrap();
    let observation = catalog_ingest::Observation::begin(&pool).await?;
    catalog_ingest::upsert_from_sc(
        &pool,
        &fields,
        catalog_ingest::TrackPriority::Like,
        catalog_ingest::TrackPriority::Like,
        observation,
    )
    .await?;
    assert!(!held(&pool, "7").await?);
    assert!(lyrics_state(&pool, "7").await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_seen_playlist_is_stored_without_scheduling_a_reconcile(
    pool: PgPool,
) -> anyhow::Result<()> {
    let playlist = json!({
        "kind": "playlist",
        "id": 9,
        "urn": "soundcloud:playlists:9",
        "title": "Night drive",
        "track_count": 12,
        "user": {"urn": "soundcloud:users:77", "username": "artist 77"},
    });
    let inserted = catalog_ingest::insert_absent_playlists(
        &pool,
        &[playlist.clone(), playlist],
        catalog_ingest::Observation::UNVERIFIED,
    )
    .await?;
    assert_eq!(
        inserted.playlists,
        vec!["soundcloud:playlists:9".to_owned()]
    );
    assert_eq!(inserted.users, vec!["77".to_owned()]);
    let states: i64 = sqlx::query_scalar("SELECT count(*) FROM playlist_membership_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(states, 0);

    let users = catalog_ingest::insert_absent_users(
        &pool,
        &[
            json!({"urn": "soundcloud:users:77", "username": "renamed"}),
            json!({"urn": "soundcloud:users:78", "username": "new"}),
        ],
        catalog_ingest::Observation::UNVERIFIED,
    )
    .await?;
    assert_eq!(users.users, vec!["78".to_owned()]);
    let name: String = sqlx::query_scalar("SELECT username FROM users WHERE sc_user_id = '77'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(name, "artist 77");
    Ok(())
}
