use super::super::edit::TrackEdit;
use super::*;

const PLAYLIST: &str = "soundcloud:playlists:42";

async fn setup(pool: &PgPool) -> anyhow::Result<PlaylistMutations> {
    catalog_ingest::upsert_playlist_from_sc(
        pool,
        &json!({
            "urn": PLAYLIST, "title": "Original", "sharing": "public", "track_count": 0,
            "user": {"urn": "soundcloud:users:17"}
        }),
        catalog_ingest::Observation::begin(pool).await?,
    )
    .await?;
    sqlx::query("INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ('17', $1)")
        .bind(PLAYLIST)
        .execute(pool)
        .await?;
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(PlaylistMutations::new(
        pool.clone(),
        SyncQueueService::new(pool.clone(), redis),
    ))
}

fn metadata(value: Value) -> anyhow::Result<PlaylistUpdate> {
    PlaylistUpdate::parse(&json!({"playlist": value})).map_err(anyhow::Error::msg)
}

async fn baseline(pool: &PgPool) -> anyhow::Result<()> {
    let snapshot: Uuid = sqlx::query_scalar("INSERT INTO playlist_remote_snapshots
        (playlist_urn, content_fingerprint, track_count) VALUES ($1, sha256(''::bytea), 0) RETURNING id")
        .bind(PLAYLIST).fetch_one(pool).await?;
    let observation: Uuid = sqlx::query_scalar("INSERT INTO playlist_remote_observations
        (playlist_urn, snapshot_id, authority, outcome, pagination_complete, all_items_identified, declared_track_count, observed_track_count)
        VALUES ($1, $2, 'owner', 'complete', true, true, 0, 0) RETURNING id")
        .bind(PLAYLIST).bind(snapshot).fetch_one(pool).await?;
    sqlx::query("UPDATE playlist_membership_state SET baseline_generation = 1, baseline_observation_id = $2,
        latest_observation_id = $2, sync_status = 'clean' WHERE playlist_urn = $1")
        .bind(PLAYLIST).bind(observation).execute(pool).await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
        VALUES ('1', 'soundcloud:tracks:1', 'Track', 'track', 120000)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn addition() -> Option<(MembershipRequest, Uuid)> {
    Some((
        MembershipRequest {
            edit: TrackEdit::Add {
                track_id: "1".into(),
            },
            expected_projection_revision: Some(0),
        },
        Uuid::now_v7(),
    ))
}

#[sqlx::test(migrations = "./migrations")]
async fn playlist_metadata_is_local_and_coalesces_without_soundcloud_or_a_membership_baseline(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    service
        .update(
            "17",
            "42",
            Some(&metadata(
                json!({"title": "Renamed", "purchase_title": "Buy"}),
            )?),
            None,
        )
        .await?;
    service
        .update(
            "17",
            PLAYLIST,
            Some(&metadata(json!({"sharing": "private"}))?),
            None,
        )
        .await?;
    let (title, sharing, desired): (String, String, Value) =
        sqlx::query_as("SELECT title, sharing, sc_desired FROM playlists WHERE urn = $1")
            .bind(PLAYLIST)
            .fetch_one(&pool)
            .await?;
    assert_eq!(title, "Renamed");
    assert_eq!(sharing, "private");
    assert_eq!(desired.get("title"), Some(&json!("Renamed")));
    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM sync_queue WHERE action_type = 'playlist_update'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        payload,
        json!({"playlist": {"title": "Renamed", "purchase_title": "Buy", "sharing": "private"}})
    );
    assert!(
        service
            .ensure_read_access("18", PLAYLIST, true)
            .await
            .is_err()
    );
    let row = super::super::PlaylistRepository::new(pool.clone())
        .find_by_urn(PLAYLIST)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing playlist"))?;
    let projected = super::super::project_to_sc_shape(&row, None);
    assert_eq!(projected.get("purchase_title"), Some(&json!("Buy")));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn missing_playlist_baseline_remains_due_after_rolling_back_a_mixed_update(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    sqlx::query(
        "UPDATE playlist_membership_state SET next_reconcile_at = NULL WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .execute(&pool)
    .await?;
    let error = service
        .update(
            "17",
            "42",
            Some(&metadata(json!({"title": "Rejected"}))?),
            addition(),
        )
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("missing baseline accepted"))?;
    assert_eq!(
        error.public_code(),
        super::super::journal::AWAITING_BASELINE
    );
    let claimed = sqlx::query_file_scalar!("queries/playlists/claim_observe_enqueue.sql", PLAYLIST)
        .fetch_optional(&pool)
        .await?;
    assert!(
        claimed.is_some(),
        "baseline observation must remain eligible for scheduling"
    );
    let result: (String, i64, i64) = sqlx::query_as(
        "SELECT title,
        (SELECT count(*) FROM sync_queue), (SELECT count(*) FROM playlist_membership_operations)
        FROM playlists WHERE urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(&pool)
    .await?;
    assert_eq!(result, ("Original".into(), 0, 0));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn playlist_metadata_and_membership_roll_back_together_on_failure(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    baseline(&pool).await?;
    let patch = metadata(json!({"title": "Blocked"}))?;
    assert!(
        service
            .update("18", PLAYLIST, Some(&patch), addition())
            .await
            .is_err()
    );
    sqlx::query("ALTER TABLE playlists ADD CHECK (title <> 'Blocked')")
        .execute(&pool)
        .await?;
    assert!(
        service
            .update("17", PLAYLIST, Some(&patch), addition())
            .await
            .is_err()
    );
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT
        (SELECT count(*) FROM sync_queue), (SELECT count(*) FROM playlist_track_projection),
        (SELECT count(*) FROM playlist_membership_operations)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(counts, (0, 0, 0));
    let outcome = service
        .update(
            "17",
            PLAYLIST,
            Some(&metadata(json!({"title": "Accepted"}))?),
            addition(),
        )
        .await?;
    assert!(outcome.metadata_queued);
    assert_eq!(outcome.journal.map(|journal| journal.appended), Some(1));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn playlist_delete_cancels_metadata_and_prevents_metadata_resurrection(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    service
        .update(
            "17",
            PLAYLIST,
            Some(&metadata(json!({"title": "Pending"}))?),
            None,
        )
        .await?;
    service.delete("17", PLAYLIST).await?;
    service.delete("17", PLAYLIST).await?;
    assert!(
        service
            .ensure_read_access("17", PLAYLIST, true)
            .await
            .is_err()
    );
    assert!(
        service
            .update(
                "17",
                PLAYLIST,
                Some(&metadata(json!({"title": "Return"}))?),
                None
            )
            .await
            .is_err()
    );
    catalog_ingest::upsert_playlist_from_sc(&pool, &json!({
        "urn": PLAYLIST, "title": "Stale", "sharing": "public", "user": {"urn": "soundcloud:users:17"}
    }), catalog_ingest::Observation::begin(&pool).await?).await?;
    let (deleted, sharing): (bool, String) =
        sqlx::query_as("SELECT deleted_at IS NOT NULL, sharing FROM playlists WHERE urn = $1")
            .bind(PLAYLIST)
            .fetch_one(&pool)
            .await?;
    assert!(deleted);
    assert_eq!(sharing, "private");
    let actions: Vec<String> = sqlx::query_scalar("SELECT action_type FROM sync_queue")
        .fetch_all(&pool)
        .await?;
    assert_eq!(actions, ["playlist_delete"]);
    let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM user_owned_playlists")
        .fetch_one(&pool)
        .await?;
    assert_eq!(owned, 0);
    let due: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT next_reconcile_at FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(&pool)
    .await?;
    assert!(due.is_none());
    Ok(())
}
