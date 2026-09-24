use sqlx::PgPool;

pub async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE playlists (
             urn text PRIMARY KEY,
             sc_playlist_id text NOT NULL DEFAULT '',
             owner_sc_user_id text,
             track_count integer NOT NULL DEFAULT 0,
             sharing text NOT NULL DEFAULT 'public',
             deleted_at timestamptz,
             desired_rev bigint NOT NULL DEFAULT 0,
             synced_rev bigint NOT NULL DEFAULT 0,
             tracks_synced_at timestamptz
         );
         CREATE TABLE playlist_tracks (
             playlist_urn text NOT NULL,
             position integer NOT NULL,
             sc_track_id text NOT NULL,
             CONSTRAINT playlist_tracks_pkey PRIMARY KEY (playlist_urn, position)
         );
         CREATE TABLE user_owned_playlists (
             user_id text NOT NULL,
             playlist_urn text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (user_id, playlist_urn)
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             sc_track_id text NOT NULL UNIQUE,
             urn text NOT NULL DEFAULT '',
             title text NOT NULL DEFAULT '',
             deleted_at timestamptz
         );
         CREATE TABLE sync_queue (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             action_type text NOT NULL,
             target_urn text NOT NULL,
             user_id text,
             generation bigint NOT NULL DEFAULT 0,
             retry_count integer NOT NULL DEFAULT 0,
             payload jsonb,
             last_error text,
             next_run_at timestamptz,
             failed_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now(),
             remote_attempted_generation bigint,
             remote_completed_generation bigint,
             remote_result jsonb
         );
         CREATE TABLE user_events (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             sc_user_id text NOT NULL,
             sc_track_id text NOT NULL,
             event_type text NOT NULL,
             weight double precision NOT NULL,
             created_at timestamp NOT NULL DEFAULT now()
         );
         CREATE TABLE disliked_tracks (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             sc_user_id text NOT NULL,
             sc_track_id text NOT NULL,
             created_at timestamp NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0077_playlist_membership_shadow.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0084_playlist_reconcile_due_index.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0085_playlist_reconcile_backoff.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0086_playlist_legacy_drain.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn seed_playlist(
    pool: &PgPool,
    playlist_urn: &str,
    owner: &str,
    track_ids: &[&str],
) -> anyhow::Result<uuid::Uuid> {
    sqlx::query(
        "INSERT INTO playlists (urn, owner_sc_user_id, track_count)
         VALUES ($1, $2, $3)",
    )
    .bind(playlist_urn)
    .bind(owner)
    .bind(track_ids.len() as i32)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ($1, $2)")
        .bind(owner)
        .bind(playlist_urn)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn, next_reconcile_at)
         VALUES ($1, clock_timestamp())",
    )
    .bind(playlist_urn)
    .execute(pool)
    .await?;
    for sc_track_id in track_ids {
        sqlx::query("INSERT INTO tracks (sc_track_id, urn, title) VALUES ($1, $2, $3)")
            .bind(sc_track_id)
            .bind(format!("soundcloud:tracks:{sc_track_id}"))
            .bind(format!("track {sc_track_id}"))
            .execute(pool)
            .await?;
    }
    let snapshot_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO playlist_remote_snapshots (playlist_urn, content_fingerprint, track_count)
         VALUES ($1, sha256($2::bytea), $3)
         RETURNING id",
    )
    .bind(playlist_urn)
    .bind(track_ids.join(",").into_bytes())
    .bind(track_ids.len() as i32)
    .fetch_one(pool)
    .await?;
    for (position, sc_track_id) in track_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_remote_snapshot_tracks (snapshot_id, position, sc_track_id)
             VALUES ($1, $2, $3)",
        )
        .bind(snapshot_id)
        .bind(position as i32)
        .bind(sc_track_id)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
             VALUES ($1, $2, $3)",
        )
        .bind(playlist_urn)
        .bind(position as i32)
        .bind(sc_track_id)
        .execute(pool)
        .await?;
    }
    let observation_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO playlist_remote_observations (
             playlist_urn, snapshot_id, authority, outcome,
             pagination_complete, all_items_identified,
             declared_track_count, observed_track_count
         )
         VALUES ($1, $2, 'owner', 'complete', true, true, $3, $3)
         RETURNING id",
    )
    .bind(playlist_urn)
    .bind(snapshot_id)
    .bind(track_ids.len() as i32)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "UPDATE playlist_membership_state
         SET baseline_generation = 1,
             baseline_observation_id = $2,
             latest_observation_id = $2,
             projection_track_count = $3,
             sync_status = 'clean'
         WHERE playlist_urn = $1",
    )
    .bind(playlist_urn)
    .bind(observation_id)
    .bind(track_ids.len() as i32)
    .execute(pool)
    .await?;
    Ok(observation_id)
}
