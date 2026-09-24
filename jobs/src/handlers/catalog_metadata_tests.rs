use catalog_ingest::{ScTrackFields, TrackPriority};
use serde_json::{Value, json};
use sqlx::PgPool;

fn track(sharing: &str) -> Value {
    json!({"urn": "soundcloud:tracks:42", "title": "Track", "sharing": sharing,
        "duration": 120000, "user": {"urn": "soundcloud:users:17"}})
}

fn playlist(sharing: &str) -> Value {
    json!({"urn": "soundcloud:playlists:42", "title": "Playlist", "sharing": sharing,
        "user": {"urn": "soundcloud:users:17"}})
}

async fn ingest_track(pool: &PgPool, value: &Value) -> anyhow::Result<()> {
    let fields = ScTrackFields::from_sc(value).ok_or_else(|| anyhow::anyhow!("invalid fixture"))?;
    catalog_ingest::upsert_from_sc(
        pool,
        &fields,
        TrackPriority::Like,
        TrackPriority::Like,
        catalog_ingest::Observation::begin(pool).await?,
    )
    .await?;
    Ok(())
}

async fn observe_track(
    pool: &PgPool,
    value: &Value,
    observation: catalog_ingest::Observation,
) -> anyhow::Result<catalog_ingest::IngestResult> {
    let fields = ScTrackFields::from_sc(value).ok_or_else(|| anyhow::anyhow!("invalid fixture"))?;
    Ok(catalog_ingest::upsert_from_sc(
        pool,
        &fields,
        TrackPriority::Like,
        TrackPriority::Like,
        observation,
    )
    .await?)
}

#[sqlx::test(migrations = false)]
async fn catalog_metadata_confirmation_does_not_admit_an_older_response(
    pool: PgPool,
) -> anyhow::Result<()> {
    use catalog_ingest::Observation;
    crate::db::migrations::run_core(&pool, None).await?;
    ingest_track(&pool, &track("public")).await?;
    let before_mutation = Observation::begin(&pool).await?;
    sqlx::query_file_scalar!(
        "../api/queries/tracks/service/apply_update.sql",
        "42",
        "17",
        &json!({"sharing": "private"})
    )
    .fetch_one(&pool)
    .await?;
    let before_ack = Observation::begin(&pool).await?;
    sqlx::query_file!(
        "queries/sync_queue/actions/confirm_track_update.sql",
        "42",
        &json!({"sharing": "private"})
    )
    .execute(&pool)
    .await?;
    assert!(
        !observe_track(&pool, &track("private"), before_ack)
            .await?
            .metadata_applied
    );
    let confirmed = Observation::begin(&pool).await?;
    assert!(
        observe_track(&pool, &track("private"), confirmed)
            .await?
            .metadata_applied
    );
    let desired: Value =
        sqlx::query_scalar("SELECT sc_desired FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(desired, json!({}));
    assert!(
        !observe_track(&pool, &track("public"), before_mutation)
            .await?
            .metadata_applied
    );
    assert!(
        !observe_track(&pool, &track("public"), before_ack)
            .await?
            .metadata_applied
    );
    assert!(
        !observe_track(&pool, &track("public"), Observation::UNVERIFIED)
            .await?
            .metadata_applied
    );
    let later = Observation::begin(&pool).await?;
    assert!(
        observe_track(&pool, &track("public"), later)
            .await?
            .metadata_applied
    );
    let sharing: String = sqlx::query_scalar("SELECT sharing FROM tracks WHERE sc_track_id = '42'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(sharing, "public");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn catalog_metadata_identical_newer_reads_advance_the_fence(
    pool: PgPool,
) -> anyhow::Result<()> {
    use catalog_ingest::Observation;
    crate::db::migrations::run_core(&pool, None).await?;
    ingest_track(&pool, &track("public")).await?;
    let older = Observation::begin(&pool).await?;
    let newer = Observation::begin(&pool).await?;
    assert!(
        observe_track(&pool, &track("public"), newer)
            .await?
            .metadata_applied
    );
    let mut stale = track("private");
    stale["duration"] = json!(9999999);
    assert!(!observe_track(&pool, &stale, older).await?.metadata_applied);
    let row: (String, i32) =
        sqlx::query_as("SELECT sharing, duration_ms FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(row, ("public".into(), 120000));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn catalog_metadata_older_soundcloud_versions_cannot_replace_newer_metadata(
    pool: PgPool,
) -> anyhow::Result<()> {
    use catalog_ingest::Observation;
    crate::db::migrations::run_core(&pool, None).await?;
    let mut current = track("public");
    current["last_modified"] = json!("2026-09-07T12:00:00Z");
    ingest_track(&pool, &current).await?;
    let mut stale = track("private");
    stale["last_modified"] = json!("2026-09-06T12:00:00Z");
    assert!(
        !observe_track(&pool, &stale, Observation::begin(&pool).await?)
            .await?
            .metadata_applied
    );
    let mut v1 = track("public");
    v1["title"] = json!("New title without optional last_modified");
    assert!(
        observe_track(&pool, &v1, Observation::begin(&pool).await?)
            .await?
            .metadata_applied
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn catalog_metadata_newer_source_versions_win_without_crossing_local_mutations(
    pool: PgPool,
) -> anyhow::Result<()> {
    use catalog_ingest::Observation;
    crate::db::migrations::run_core(&pool, None).await?;
    let mut current = track("public");
    current["last_modified"] = json!("2026-09-06T12:00:00Z");
    ingest_track(&pool, &current).await?;
    let slower = Observation::begin(&pool).await?;
    let faster = Observation::begin(&pool).await?;
    assert!(
        observe_track(&pool, &current, faster)
            .await?
            .metadata_applied
    );
    current["title"] = json!("Newest remote version");
    current["last_modified"] = json!("2026-09-07T12:00:00Z");
    assert!(
        observe_track(&pool, &current, slower)
            .await?
            .metadata_applied
    );
    let before_mutation = Observation::begin(&pool).await?;
    sqlx::query_file_scalar!(
        "../api/queries/tracks/service/apply_update.sql",
        "42",
        "17",
        &json!({"sharing": "private"})
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query_file!(
        "queries/sync_queue/actions/confirm_track_update.sql",
        "42",
        &json!({"sharing": "private"})
    )
    .execute(&pool)
    .await?;
    current["sharing"] = json!("private");
    let confirmation = Observation::begin(&pool).await?;
    assert!(
        observe_track(&pool, &current, confirmation)
            .await?
            .metadata_applied
    );
    current["last_modified"] = json!("2026-09-08T12:00:00Z");
    current["sharing"] = json!("public");
    assert!(
        !observe_track(&pool, &current, before_mutation)
            .await?
            .metadata_applied
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn catalog_metadata_refresh_cannot_publish_pending_local_private_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    ingest_track(&pool, &track("public")).await?;
    sqlx::query_file_scalar!(
        "../api/queries/tracks/service/apply_update.sql",
        "42",
        "17",
        &json!({"sharing": "private"})
    )
    .fetch_one(&pool)
    .await?;
    ingest_track(&pool, &track("public")).await?;
    let sharing: String = sqlx::query_scalar("SELECT sharing FROM tracks WHERE sc_track_id = '42'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(sharing, "private");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn catalog_metadata_refresh_cannot_publish_acknowledged_local_private_playlists(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    catalog_ingest::upsert_playlist_from_sc(
        &pool,
        &playlist("public"),
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    sqlx::query_file_scalar!(
        "../api/queries/playlists/service/apply_update.sql",
        "soundcloud:playlists:42",
        "17",
        &json!({"sharing": "private"})
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query_file!(
        "queries/sync_queue/actions/confirm_playlist_update.sql",
        "soundcloud:playlists:42",
        &json!({"sharing": "private"})
    )
    .execute(&pool)
    .await?;
    catalog_ingest::upsert_playlist_from_sc(
        &pool,
        &playlist("public"),
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    let sharing: String =
        sqlx::query_scalar("SELECT sharing FROM playlists WHERE urn = 'soundcloud:playlists:42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(sharing, "private");
    Ok(())
}
