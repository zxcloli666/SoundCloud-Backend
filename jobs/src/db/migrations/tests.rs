use sqlx::PgPool;
use uuid::Uuid;

use super::*;

const LEGACY_SCHEMA_VERSION: i64 = CONNECTION_MIGRATION - 1;
const LATEST_SCHEMA_VERSION: i64 = 115;

#[sqlx::test(migrations = false)]
async fn legacy_playlist_update_is_archived_before_sharing_becomes_metadata_update(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(67).run(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id)
         VALUES ('soundcloud:playlists:42', '42', 'Playlist', 'playlist', '17');
         INSERT INTO sync_queue (user_id, action_type, target_urn, payload)
         VALUES ('17', 'playlist_update', 'soundcloud:playlists:42', '{\"playlist\":{\"tracks\":[]}}'),
                ('17', 'playlist_sharing', 'soundcloud:playlists:42', '{\"sharing\":\"private\"}')",
    ).execute(&pool).await?;
    run_core(&pool, None).await?;
    let actions: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT action_type, payload FROM sync_queue WHERE target_urn = 'soundcloud:playlists:42'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        actions,
        vec![(
            "playlist_update".into(),
            serde_json::json!({"playlist":{"sharing":"private"}})
        )]
    );
    let archived: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM playlist_legacy_membership_intents WHERE playlist_urn = 'soundcloud:playlists:42'",
    ).fetch_one(&pool).await?;
    assert_eq!(archived, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn pending_sharing_intents_survive_metadata_fences_and_track_action_conversion(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(91).run(&pool).await?;
    sqlx::raw_sql("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
        VALUES ('42', 'soundcloud:tracks:42', 'Track', 'track', 120000, '17');
        INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id)
        VALUES ('soundcloud:playlists:43', '43', 'Playlist', 'playlist', '17');
        INSERT INTO sync_queue (user_id, action_type, target_urn, payload, dead)
        VALUES ('17', 'track_sharing', 'soundcloud:tracks:42', '{\"sharing\":\"private\"}', true),
               ('17', 'playlist_sharing', 'soundcloud:playlists:43', '{\"sharing\":\"private\"}', false)")
        .execute(&pool).await?;
    run_core(&pool, None).await?;
    let track: (String, serde_json::Value, i64) = sqlx::query_as(
        "SELECT sharing, sc_desired, sc_mutation_observation FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(track.0, "private");
    assert_eq!(track.1, serde_json::json!({"sharing": "private"}));
    assert!(track.2 > 0);
    let playlist: (String, i64) = sqlx::query_as("SELECT sharing, sc_mutation_observation FROM playlists WHERE urn = 'soundcloud:playlists:43'")
        .fetch_one(&pool).await?;
    assert_eq!(playlist.0, "private");
    assert!(playlist.1 > 0);
    let queued: (String, serde_json::Value, i64, bool) = sqlx::query_as("SELECT action_type, payload, generation, dead FROM sync_queue WHERE target_urn = 'soundcloud:tracks:42'")
        .fetch_one(&pool).await?;
    assert_eq!(
        queued,
        (
            "track_update".into(),
            serde_json::json!({"track": {"sharing": "private"}}),
            2,
            true
        )
    );
    let playlist_queue: (String, serde_json::Value, i64) = sqlx::query_as("SELECT action_type, payload, generation FROM sync_queue WHERE target_urn = 'soundcloud:playlists:43'")
        .fetch_one(&pool).await?;
    assert_eq!(
        playlist_queue,
        (
            "playlist_update".into(),
            serde_json::json!({"playlist": {"sharing": "private"}}),
            2
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn queued_playlist_deletion_installs_a_tombstone_and_stops_observation(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(93).run(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id)
        VALUES ('soundcloud:playlists:42', '42', 'Playlist', 'playlist', '17');
        INSERT INTO playlist_membership_state (playlist_urn, next_reconcile_at)
        VALUES ('soundcloud:playlists:42', now());
        INSERT INTO sync_queue (user_id, action_type, target_urn)
        VALUES ('17', 'playlist_delete', 'soundcloud:playlists:42')",
    )
    .execute(&pool)
    .await?;
    run_core(&pool, None).await?;
    let row: (bool, String, i64, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT p.deleted_at IS NOT NULL, p.sharing, p.sc_mutation_observation, s.next_reconcile_at
        FROM playlists p JOIN playlist_membership_state s ON s.playlist_urn = p.urn",
    )
    .fetch_one(&pool)
    .await?;
    assert!(row.0);
    assert_eq!(row.1, "private");
    assert!(row.2 > 0);
    assert!(row.3.is_none());
    Ok(())
}

fn configured_app() -> OAuthAppBootstrap {
    OAuthAppBootstrap {
        name: "default".to_owned(),
        client_id: "configured-client".to_owned(),
        client_secret: "configured-secret".to_owned().into(),
        redirect_uri: "https://localhost/callback".to_owned(),
    }
}

async fn install_legacy_schema(pool: &PgPool) -> anyhow::Result<()> {
    core_through(LEGACY_SCHEMA_VERSION).run(pool).await?;
    Ok(())
}

async fn insert_environment_session(pool: &PgPool) -> anyhow::Result<Uuid> {
    let session_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (
             id, access_token, refresh_token, expires_at, scope,
             soundcloud_user_id, username, oauth_app_id
         ) VALUES ($1, 'access', 'refresh', now() + interval '1 hour', '',
                   'soundcloud:users:42', 'listener', NULL)",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(session_id)
}

async fn insert_oauth_app(pool: &PgPool, client_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO oauth_apps (
             id, name, client_id, client_secret, redirect_uri
         ) VALUES ($1, $2, $3, 'secret', 'https://localhost/callback')",
    )
    .bind(Uuid::now_v7())
    .bind(format!("app-{client_id}"))
    .bind(client_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn latest_version(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(pool)
            .await?,
    )
}

async fn legacy_column_exists(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'soundcloud_connections'
               AND column_name = 'uses_environment_oauth_app'
         )",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn fresh_database_reaches_latest_schema_without_configured_app(
    pool: PgPool,
) -> anyhow::Result<()> {
    run_core(&pool, None).await?;

    assert_eq!(latest_version(&pool).await?, LATEST_SCHEMA_VERSION);
    assert!(!legacy_column_exists(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn transcription_migration_quarantines_legacy_pending_work(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(73).run(&pool).await?;
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             transcribe_state, transcribe_at
         ) VALUES
             ('pending', 'soundcloud:tracks:1', 'pending', 'pending', 120000,
              'pending', now() - interval '5 minutes'),
             ('done', 'soundcloud:tracks:2', 'done', 'done', 120000,
              'done', now() - interval '4 minutes'),
             ('disabled', 'soundcloud:tracks:3', 'disabled', 'disabled', 120000,
              'disabled', now() - interval '3 minutes')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at,
             uploaded_generation, transcription_generation
         ) VALUES ('pending', 'STORAGE_EVENTS', 1, now(), 2, 2)",
    )
    .execute(&pool)
    .await?;

    core_through(74).run(&pool).await?;

    let wire_state = sqlx::query_as::<_, (String, String, Option<i64>, Option<String>)>(
        "SELECT sc_track_id, status, upload_generation, quarantine_reason
         FROM transcription_wire_state
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        wire_state,
        vec![
            ("disabled".to_owned(), "empty".to_owned(), None, None),
            ("done".to_owned(), "done".to_owned(), None, None),
            (
                "pending".to_owned(),
                "quarantined".to_owned(),
                Some(2),
                Some("legacy_pending_at_cutover".to_owned()),
            ),
        ]
    );
    let track_states = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT sc_track_id, transcribe_state
         FROM tracks
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        track_states,
        vec![
            ("disabled".to_owned(), Some("disabled".to_owned())),
            ("done".to_owned(), Some("done".to_owned())),
            ("pending".to_owned(), Some("quarantined".to_owned())),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn audio_index_migration_adopts_only_already_indexed_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(87).run(&pool).await?;
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             index_state, indexed_at
         ) VALUES
             ('indexed', 'soundcloud:tracks:1', 'indexed', 'indexed', 120000,
              'indexed', now() - interval '5 minutes'),
             ('pending', 'soundcloud:tracks:2', 'pending', 'pending', 120000,
              'pending', NULL),
             ('orphan', 'soundcloud:tracks:3', 'orphan', 'orphan', 120000,
              'indexed', now() - interval '3 minutes'),
             ('rejected', 'soundcloud:tracks:4', 'rejected', 'rejected', 120000,
              'indexed', now() - interval '2 minutes')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ('indexed', 'STORAGE_EVENTS', 1, now(), 3),
                  ('rejected', 'STORAGE_EVENTS', 2, now(), 0)",
    )
    .execute(&pool)
    .await?;

    core_through(88).run(&pool).await?;

    let wire_state = sqlx::query_as::<_, (String, String, Option<i64>)>(
        "SELECT sc_track_id, status, upload_generation
         FROM audio_index_wire_state
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        wire_state,
        vec![
            ("indexed".to_owned(), "done".to_owned(), Some(3)),
            ("orphan".to_owned(), "done".to_owned(), None),
            ("rejected".to_owned(), "done".to_owned(), None),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn lyrics_recovery_migration_fences_legacy_rows_without_a_backfill(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(74).run(&pool).await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (
             sc_track_id, plain_text, source, embedded_at
         ) VALUES
             ('embedded', 'already embedded lyrics', 'genius', now()),
             ('pending', 'unattributed legacy embedding work', 'genius', NULL)",
    )
    .execute(&pool)
    .await?;

    core_through(75).run(&pool).await?;

    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('new', 'post-migration lyrics', 'genius')",
    )
    .execute(&pool)
    .await?;
    let cache = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT sc_track_id, embedding_state
         FROM lyrics_cache
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        cache,
        vec![
            ("embedded".to_owned(), Some("legacy".to_owned())),
            ("new".to_owned(), None),
            ("pending".to_owned(), Some("legacy".to_owned())),
        ]
    );
    let wire_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_embedding_wire_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(wire_rows, 0);

    let invalid_pending = sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (sc_track_id, status)
         VALUES ('invalid', 'pending')",
    )
    .execute(&pool)
    .await;
    assert!(invalid_pending.is_err());

    let partial_result_identity = sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, completed_at, result_consumer
         ) VALUES ('partial-identity', 'done', now(), 'consumer')",
    )
    .execute(&pool)
    .await;
    assert!(partial_result_identity.is_err());

    let result_identity_without_kind = sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, completed_at,
             result_consumer, result_stream,
             result_stream_sequence, result_published_at
         ) VALUES (
             'missing-result-kind', 'done', now(),
             'consumer', 'stream', 1, now()
         )",
    )
    .execute(&pool)
    .await;
    assert!(result_identity_without_kind.is_err());

    let partial_result_lease = sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, lyrics_created_at, request_version,
             request_text, request_sha256, request_message_id,
             first_publish_attempt_at, publish_retry_until,
             result_consumer, result_stream, result_kind,
             result_stream_sequence, result_published_at, result_lease_id
         ) VALUES (
             'partial-lease', 'pending', timestamp '2026-01-01', 1,
             'lyrics', decode(repeat('00', 32), 'hex'), 'request',
             now(), now() + interval '1 hour',
             'consumer', 'stream', 'vector', 1, now(),
             '0198b6dc-b6d0-7000-8000-000000000001'::uuid
         )",
    )
    .execute(&pool)
    .await;
    assert!(partial_result_lease.is_err());

    let mismatched_terminal_kind = sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, completed_at,
             result_consumer, result_stream, result_kind,
             result_stream_sequence, result_published_at
         ) VALUES (
             'mismatched-terminal', 'skipped', now(),
             'consumer', 'stream', 'vector', 1, now()
         )",
    )
    .execute(&pool)
    .await;
    assert!(mismatched_terminal_kind.is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn playlist_sync_cutover_archives_legacy_work_and_removes_old_authority(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(75).run(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn,
             lease_id, lease_generation, locked_at
         ) VALUES (
             '1', 'playlist_sync', 'soundcloud:playlists:blocked',
             $1, 1, now()
         )",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn,
             dead, failed_at, last_error, next_run_at,
             remote_attempted_generation, remote_completed_generation,
             remote_result
         ) VALUES (
             '1', 'playlist_sync', 'soundcloud:playlists:completed',
             true, now(), 'old failure', 'infinity', 1, 1,
             '{\"revision\":1}'
         )",
    )
    .execute(&pool)
    .await?;

    core_through(76).run(&pool).await?;

    core_through(77).run(&pool).await?;

    let queued = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sync_queue WHERE action_type = 'playlist_sync'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(queued, 0);

    let archived = sqlx::query_as::<_, (String, Option<i64>, bool)>(
        "SELECT playlist_urn, remote_completed_generation, remote_result IS NOT NULL
         FROM playlist_legacy_membership_intents
         WHERE source = 'queue'
         ORDER BY playlist_urn",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        archived,
        vec![
            ("soundcloud:playlists:blocked".to_owned(), None, false),
            ("soundcloud:playlists:completed".to_owned(), Some(1), true,),
        ]
    );

    let legacy_columns = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'playlists'
           AND column_name IN ('desired_rev', 'synced_rev', 'tracks_synced_at')",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(legacy_columns, 0);
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT to_regclass('playlist_track_projection') IS NOT NULL
                 AND to_regclass('playlist_tracks') IS NULL",
        )
        .fetch_one(&pool)
        .await?
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn legacy_environment_connection_is_bound_before_source_column_is_removed(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_legacy_schema(&pool).await?;
    let session_id = insert_environment_session(&pool).await?;

    run_core(&pool, Some(&configured_app())).await?;

    let stored = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT connection.oauth_app_id,
                app.client_id,
                app.client_secret,
                app.redirect_uri
         FROM sessions AS session
         JOIN soundcloud_connections AS connection
           ON connection.id = session.soundcloud_connection_id
         JOIN oauth_apps AS app ON app.id = connection.oauth_app_id
         WHERE session.id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;

    assert_ne!(stored.0, Uuid::nil());
    assert_eq!(stored.1, "configured-client");
    assert_eq!(stored.2, "configured-secret");
    assert_eq!(stored.3, "https://localhost/callback");
    assert_eq!(latest_version(&pool).await?, LATEST_SCHEMA_VERSION);
    assert!(!legacy_column_exists(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn legacy_environment_connection_blocks_finalization_without_configured_app(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_legacy_schema(&pool).await?;
    let session_id = insert_environment_session(&pool).await?;

    let error = run_core(&pool, None).await.unwrap_err();

    assert!(matches!(
        error,
        MigrationError::LegacyOAuthAppNotConfigured { connections: 1 }
    ));
    assert_eq!(latest_version(&pool).await?, CONNECTION_MIGRATION);
    assert!(legacy_column_exists(&pool).await?);
    let session_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(session_id)
            .fetch_one(&pool)
            .await?;
    assert!(session_exists);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn migration_waits_for_the_advisory_lock_past_the_pool_lock_timeout(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lock = 0x5343_445F_5445_i64;
    let mut blocker = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock)
        .execute(&mut *blocker)
        .await?;

    let waiter_pool = pool.clone();
    let waiting = tokio::spawn(async move {
        let mut connection = waiter_pool.acquire().await?;
        sqlx::query("SET lock_timeout = '50ms'")
            .execute(&mut *connection)
            .await?;
        prepare(&mut connection, lock).await?;
        finish(&mut connection, lock, Ok(())).await
    });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock)
        .execute(&mut *blocker)
        .await?;

    tokio::time::timeout(std::time::Duration::from_secs(2), waiting).await???;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn existing_interest_index_does_not_block_schema_finalization(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(64).run(&pool).await?;
    sqlx::query(
        "CREATE INDEX artists_positive_interest_idx
         ON artists (id)
         WHERE interest_score > 0",
    )
    .execute(&pool)
    .await?;

    run_core(&pool, None).await?;

    assert_eq!(latest_version(&pool).await?, LATEST_SCHEMA_VERSION);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn existing_app_token_gets_a_deterministic_initial_generation(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(63).run(&pool).await?;
    let app_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (
             id, name, client_id, client_secret, redirect_uri
         ) VALUES ($1, 'test', 'client', 'secret', 'https://localhost/callback')",
    )
    .bind(app_id)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, access_token, expires_at)
         VALUES ($1, 'token', now() + interval '1 hour')",
    )
    .bind(app_id)
    .execute(&pool)
    .await?;

    run_core(&pool, None).await?;

    let generation: Uuid =
        sqlx::query_scalar("SELECT generation FROM oauth_app_tokens WHERE oauth_app_id = $1")
            .bind(app_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(generation, app_id);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn oauth_identity_migration_refuses_legacy_duplicates(pool: PgPool) -> anyhow::Result<()> {
    core_through(66).run(&pool).await?;
    insert_oauth_app(&pool, "client").await?;
    insert_oauth_app(&pool, " client ").await?;

    let error = core_through(67).run(&pool).await.unwrap_err();
    let sqlx::migrate::MigrateError::ExecuteMigration(sqlx::Error::Database(error), 67) = error
    else {
        anyhow::bail!("unexpected migration error: {error:?}");
    };

    assert_eq!(error.code().as_deref(), Some("23505"));
    assert!(error.message().contains("migration 0067 refused"));
    let index_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('oauth_apps_client_identity_uq') IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert!(!index_exists);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn oauth_identity_migration_refuses_blank_identity(pool: PgPool) -> anyhow::Result<()> {
    core_through(66).run(&pool).await?;
    insert_oauth_app(&pool, "\u{3000}\u{00a0}").await?;

    let error = core_through(67).run(&pool).await.unwrap_err();
    let sqlx::migrate::MigrateError::ExecuteMigration(sqlx::Error::Database(error), 67) = error
    else {
        anyhow::bail!("unexpected migration error: {error:?}");
    };

    assert_eq!(error.code().as_deref(), Some("23505"));
    assert!(error.message().contains("migration 0067 refused"));
    let index_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('oauth_apps_client_identity_uq') IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert!(!index_exists);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn oauth_identity_preflight_stops_before_bootstrap_and_remap(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_legacy_schema(&pool).await?;
    let session_id = insert_environment_session(&pool).await?;
    insert_oauth_app(&pool, "configured-client").await?;
    insert_oauth_app(&pool, " configured-client ").await?;
    insert_oauth_app(&pool, "\u{3000}\u{00a0}").await?;

    let error = run_core(&pool, Some(&configured_app())).await.unwrap_err();
    let MigrationError::InvalidOAuthAppIdentities { groups, sample } = error else {
        anyhow::bail!("unexpected migration error: {error:?}");
    };

    assert_eq!(groups, 2);
    assert!(sample.contains("configured-client"));
    assert!(sample.contains("'' =>"));
    let configured_credentials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM oauth_apps WHERE client_secret = 'configured-secret'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(configured_credentials, 0);
    let connection_state: (Option<Uuid>, bool) = sqlx::query_as(
        "SELECT connection.oauth_app_id, connection.uses_environment_oauth_app
         FROM sessions AS session
         JOIN soundcloud_connections AS connection
           ON connection.id = session.soundcloud_connection_id
         WHERE session.id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(connection_state, (None, true));
    assert_eq!(latest_version(&pool).await?, CONNECTION_MIGRATION);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn oauth_identity_migration_matches_rust_unicode_trim(pool: PgPool) -> anyhow::Result<()> {
    core_through(66).run(&pool).await?;
    insert_oauth_app(&pool, "client").await?;
    core_through(67).run(&pool).await?;

    let error = insert_oauth_app(&pool, "\u{3000}client\u{00a0}")
        .await
        .expect_err("Unicode-trimmed duplicate must be rejected");
    let error = error
        .as_database_error()
        .expect("duplicate identity must be a database error");

    assert_eq!(error.code().as_deref(), Some("23505"));
    Ok(())
}
