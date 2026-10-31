use serde_json::json;
use sqlx::PgPool;

pub async fn seed_playlist(
    pool: &PgPool,
    playlist_urn: &str,
    owner: &str,
    track_ids: &[&str],
) -> anyhow::Result<uuid::Uuid> {
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, track_count)
         VALUES ($1, $1, 'Mix', 'mix', $2, $3)",
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
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ($1, $2, $3, $3, 120000)",
        )
        .bind(sc_track_id)
        .bind(format!("soundcloud:tracks:{sc_track_id}"))
        .bind(format!("track {sc_track_id}"))
        .execute(pool)
        .await?;
    }
    baseline(pool, playlist_urn, track_ids).await
}

pub async fn observe(
    pool: &PgPool,
    playlist_urn: &str,
    owner: &str,
    track_count: i32,
) -> anyhow::Result<()> {
    catalog_ingest::upsert_playlist_from_sc(
        pool,
        &json!({
            "urn": playlist_urn, "title": "Mix", "sharing": "public", "track_count": track_count,
            "user": {"urn": format!("soundcloud:users:{owner}")}
        }),
        catalog_ingest::Observation::begin(pool).await?,
    )
    .await?;
    Ok(())
}

pub async fn seed_observed_playlist(
    pool: &PgPool,
    playlist_urn: &str,
    owner: &str,
    track_ids: &[&str],
) -> anyhow::Result<uuid::Uuid> {
    observe(pool, playlist_urn, owner, track_ids.len() as i32).await?;
    sqlx::query("INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ($1, $2)")
        .bind(owner)
        .bind(playlist_urn)
        .execute(pool)
        .await?;
    for sc_track_id in track_ids {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ($1, $2, $3, $3, 120000)",
        )
        .bind(sc_track_id)
        .bind(format!("soundcloud:tracks:{sc_track_id}"))
        .bind(format!("track {sc_track_id}"))
        .execute(pool)
        .await?;
    }
    baseline(pool, playlist_urn, track_ids).await
}

async fn baseline(
    pool: &PgPool,
    playlist_urn: &str,
    track_ids: &[&str],
) -> anyhow::Result<uuid::Uuid> {
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
