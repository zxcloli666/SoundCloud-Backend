use std::borrow::Cow;

use sqlx::PgPool;
use uuid::Uuid;

static CORE: sqlx::migrate::Migrator = {
    let mut migrator = sqlx::migrate!("../api/migrations");
    migrator.ignore_missing = true;
    migrator
};

fn core_through(version: i64) -> sqlx::migrate::Migrator {
    sqlx::migrate::Migrator {
        migrations: Cow::Owned(
            CORE.iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: CORE.ignore_missing,
        locking: CORE.locking,
        no_tx: CORE.no_tx,
    }
}

async fn insert_playlist(
    pool: &PgPool,
    id: &str,
    desired_revision: i64,
    synced_revision: i64,
    declared_track_count: i32,
) -> anyhow::Result<String> {
    let urn = format!("soundcloud:playlists:{id}");
    sqlx::query(
        "INSERT INTO playlists (
             urn, sc_playlist_id, title, title_normalized, track_count,
             desired_rev, synced_rev, tracks_synced_at
         ) VALUES ($1, $2, $3, $3, $4, $5, $6, now())",
    )
    .bind(&urn)
    .bind(id)
    .bind(format!("playlist-{id}"))
    .bind(declared_track_count)
    .bind(desired_revision)
    .bind(synced_revision)
    .execute(pool)
    .await?;
    Ok(urn)
}

#[sqlx::test(migrations = false)]
async fn cutover_preserves_projection_and_quarantines_every_legacy_intent(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(76).run(&pool).await?;
    let queued = insert_playlist(&pool, "queued", 4, 2, 28).await?;
    let revision_only = insert_playlist(&pool, "revision", 3, 1, 9).await?;
    let clean = insert_playlist(&pool, "clean", 7, 7, 1).await?;

    sqlx::query(
        "INSERT INTO playlist_tracks (playlist_urn, position, sc_track_id)
         VALUES
             ($1, 0, '11'),
             ($1, 1, '12'),
             ($2, 0, '21')",
    )
    .bind(&queued)
    .bind(&clean)
    .execute(&pool)
    .await?;

    let queued_id = Uuid::now_v7();
    let orphan_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sync_queue (
             id, user_id, action_type, target_urn, retry_count, last_error,
             remote_attempted_generation, remote_completed_generation, remote_result
         ) VALUES
             ($1, '42', 'playlist_sync', $2, 3, 'upstream timeout', 1, 1,
              '{\"revision\":4}'),
             ($3, '43', 'playlist_sync', 'soundcloud:playlists:missing', 2,
              'missing playlist', NULL, NULL, NULL)",
    )
    .bind(queued_id)
    .bind(&queued)
    .bind(orphan_id)
    .execute(&pool)
    .await?;

    core_through(77).run(&pool).await?;

    let old_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('playlist_tracks')::text")
            .fetch_one(&pool)
            .await?;
    assert_eq!(old_table, None);

    let projection = sqlx::query_as::<_, (String, i32, String)>(
        "SELECT playlist_urn, position, sc_track_id
         FROM playlist_track_projection
         ORDER BY playlist_urn, position",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        projection,
        vec![
            (clean.clone(), 0, "21".to_owned()),
            (queued.clone(), 0, "11".to_owned()),
            (queued.clone(), 1, "12".to_owned()),
        ]
    );

    let state = sqlx::query_as::<_, (String, i64, i32, String)>(
        "SELECT playlist_urn, projection_revision, projection_track_count, sync_status
         FROM playlist_membership_state
         ORDER BY playlist_urn",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        state,
        vec![
            (clean.clone(), 7, 1, "unhydrated".to_owned()),
            (queued.clone(), 4, 2, "legacy_review".to_owned()),
            (revision_only.clone(), 3, 0, "legacy_review".to_owned()),
        ]
    );

    let archived = sqlx::query_as::<_, (Uuid, Option<Uuid>, String, Option<i64>, Option<i64>)>(
        "SELECT archive_id, queue_id, playlist_urn,
                legacy_desired_revision, legacy_synced_revision
         FROM playlist_legacy_membership_intents
         ORDER BY playlist_urn",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(archived.len(), 3);
    assert!(archived.iter().any(|row| {
        row.0 == orphan_id
            && row.1 == Some(orphan_id)
            && row.2 == "soundcloud:playlists:missing"
            && row.3.is_none()
            && row.4.is_none()
    }));
    assert!(archived.iter().any(|row| {
        row.0 == queued_id
            && row.1 == Some(queued_id)
            && row.2 == queued
            && row.3 == Some(4)
            && row.4 == Some(2)
    }));
    assert!(archived.iter().any(|row| {
        row.1.is_none() && row.2 == revision_only && row.3 == Some(3) && row.4 == Some(1)
    }));

    let queued_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sync_queue WHERE action_type = 'playlist_sync'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(queued_rows, 0);

    let legacy_columns: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'playlists'
           AND column_name = ANY(ARRAY['desired_rev', 'synced_rev', 'tracks_synced_at'])",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(legacy_columns, 0);

    let duplicate = sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ($1, 2, '11')",
    )
    .bind(&queued)
    .execute(&pool)
    .await;
    assert!(duplicate.is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn duplicate_legacy_membership_aborts_before_rename_or_archive(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(76).run(&pool).await?;
    let urn = insert_playlist(&pool, "duplicate", 1, 0, 2).await?;
    sqlx::query(
        "INSERT INTO playlist_tracks (playlist_urn, position, sc_track_id)
         VALUES ($1, 0, '11'), ($1, 1, '11')",
    )
    .bind(&urn)
    .execute(&pool)
    .await?;

    let error = core_through(77).run(&pool).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate playlist membership must be repaired before migration")
    );

    let old_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('playlist_tracks')::text")
            .fetch_one(&pool)
            .await?;
    let new_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('playlist_track_projection')::text")
            .fetch_one(&pool)
            .await?;
    assert_eq!(old_table.as_deref(), Some("playlist_tracks"));
    assert_eq!(new_table, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn shadow_schema_rejects_incomplete_baselines_and_invalid_operations(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(77).run(&pool).await?;
    let urn = "soundcloud:playlists:new";
    sqlx::query(
        "INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized)
         VALUES ($1, 'new', 'new', 'new')",
    )
    .bind(urn)
    .execute(&pool)
    .await?;
    let snapshot_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO playlist_remote_snapshots (
             id, playlist_urn, content_fingerprint, track_count
         ) VALUES ($1, $2, decode(repeat('01', 32), 'hex'), 1)",
    )
    .bind(snapshot_id)
    .bind(urn)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_remote_snapshot_tracks (snapshot_id, position, sc_track_id)
         VALUES ($1, 0, '11')",
    )
    .bind(snapshot_id)
    .execute(&pool)
    .await?;

    let incomplete = sqlx::query(
        "INSERT INTO playlist_remote_observations (
             playlist_urn, snapshot_id, authority, outcome,
             pagination_complete, all_items_identified,
             declared_track_count, observed_track_count
         ) VALUES ($1, $2, 'owner', 'complete', true, true, 2, 1)",
    )
    .bind(urn)
    .bind(snapshot_id)
    .execute(&pool)
    .await;
    assert!(incomplete.is_err());

    let observation_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO playlist_remote_observations (
             id, playlist_urn, snapshot_id, authority, outcome,
             pagination_complete, all_items_identified,
             declared_track_count, observed_track_count
         ) VALUES ($1, $2, $3, 'owner', 'complete', true, true, 1, 1)",
    )
    .bind(observation_id)
    .bind(urn)
    .bind(snapshot_id)
    .execute(&pool)
    .await?;
    let eligible: bool =
        sqlx::query_scalar("SELECT write_eligible FROM playlist_remote_observations WHERE id = $1")
            .bind(observation_id)
            .fetch_one(&pool)
            .await?;
    assert!(eligible);

    sqlx::query(
        "INSERT INTO playlist_membership_state (
             playlist_urn, baseline_generation, baseline_observation_id,
             latest_observation_id, sync_status
         ) VALUES ($1, 1, $2, $2, 'clean')",
    )
    .bind(urn)
    .bind(observation_id)
    .execute(&pool)
    .await?;

    let invalid_operation = sqlx::query(
        "INSERT INTO playlist_membership_operations (
             operation_id, playlist_urn, sequence, actor_sc_user_id,
             idempotency_key, request_fingerprint,
             base_baseline_generation, base_observation_id,
             expected_projection_revision, accepted_projection_revision,
             kind, track_id
         ) VALUES (
             $1, $2, 1, '42', $3, decode(repeat('02', 32), 'hex'),
             1, $4, 0, 1, 'move', '11'
         )",
    )
    .bind(Uuid::now_v7())
    .bind(urn)
    .bind(Uuid::now_v7())
    .bind(observation_id)
    .execute(&pool)
    .await;
    assert!(invalid_operation.is_err());
    Ok(())
}
