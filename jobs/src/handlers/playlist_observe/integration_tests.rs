use std::sync::Arc;
use std::time::Duration;

use backend_contracts::PlaylistObservePayload;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};
use tokio::time::timeout;
use url::Url;
use uuid::Uuid;

use crate::config::{OAuthConfig, PlaylistReconcileConfig, SyncQueueConfig};
use crate::queue::JobRepository;

use super::client::PlaylistReadClient;
use super::remote::PlaylistReader;
use super::repository::PlaylistObserveRepository;
use super::{ConnectionManager, PlaylistObserveHandler, TokenRefreshClient};

#[sqlx::test(migrations = false)]
async fn observer_is_get_only_and_releases_a_single_connection_before_http(
    setup_pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&setup_pool).await?;
    seed_playlist_and_connection(&setup_pool).await?;
    let pool = single_connection_pool(&setup_pool).await?;
    let (api_url, first_request, release_response, methods, server) = mock_soundcloud().await?;
    let sync = sync_config(api_url.clone());
    let oauth = OAuthConfig {
        token_url: api_url.join("oauth/token")?,
        bootstrap_app: None,
    };
    let handler = PlaylistObserveHandler {
        pool: pool.clone(),
        queue: JobRepository::new(pool.clone(), "playlist-observe-test".to_owned()),
        repository: PlaylistObserveRepository::new(pool.clone(), false),
        connections: ConnectionManager::new(pool.clone()),
        token_client: TokenRefreshClient::new(&oauth)?,
        reader: PlaylistReader::new(PlaylistReadClient::new(&sync)?),
        reconcile: PlaylistReconcileConfig {
            sweep_batch: 512,
            sweep_owner_share: 8,
            claim_seconds: 600,
            legacy_drain_batch: 500,
            membership_remote_apply: false,
        },
    };
    let job = tokio::spawn(async move {
        handler
            .observe(
                Uuid::now_v7(),
                1,
                PlaylistObservePayload {
                    playlist_urn: "soundcloud:playlists:42".to_owned(),
                },
            )
            .await
    });

    timeout(Duration::from_secs(5), first_request).await??;
    let connection = timeout(Duration::from_secs(1), pool.acquire()).await??;
    drop(connection);
    release_response
        .send(())
        .map_err(|_| anyhow::anyhow!("mock response receiver was dropped"))?;
    job.await?
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    server.await??;

    let methods = methods.lock().await.clone();
    assert_eq!(methods, ["GET", "GET", "GET"]);
    let state = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            i32,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        "SELECT sync_status, conflict_code, projection_track_count, next_reconcile_at
         FROM playlist_membership_state
         WHERE playlist_urn = 'soundcloud:playlists:42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state.0, "conflict");
    assert_eq!(state.1.as_deref(), Some("legacy_remote_superset"));
    assert_eq!(state.2, 28);
    assert!(state.3.is_some_and(|due| {
        due > chrono::Utc::now() && due < chrono::Utc::now() + chrono::Duration::minutes(16)
    }));
    let projection = sqlx::query_scalar::<_, String>(
        "SELECT sc_track_id
         FROM playlist_track_projection
         WHERE playlist_urn = 'soundcloud:playlists:42'
         ORDER BY position",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(projection, track_ids());
    let catalog_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
         FROM playlist_track_projection AS projection
         JOIN tracks ON tracks.sc_track_id = projection.sc_track_id
         WHERE projection.playlist_urn = 'soundcloud:playlists:42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(catalog_count, 28);
    let observation = sqlx::query_as::<_, (String, bool, bool)>(
        "SELECT outcome, write_eligible, pagination_complete
         FROM playlist_remote_observations
         WHERE playlist_urn = 'soundcloud:playlists:42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(observation, ("complete".to_owned(), true, true));
    let legacy = sqlx::query_scalar::<_, String>(
        "SELECT classification
         FROM playlist_legacy_membership_intents
         WHERE playlist_urn = 'soundcloud:playlists:42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(legacy, "remote_superset");
    Ok(())
}

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE EXTENSION IF NOT EXISTS pgcrypto;
         CREATE TABLE playlists (
             urn text PRIMARY KEY,
             owner_sc_user_id text,
             sharing text NOT NULL DEFAULT 'public',
             track_count integer NOT NULL DEFAULT 0,
             desired_rev bigint NOT NULL DEFAULT 0,
             synced_rev bigint NOT NULL DEFAULT 0,
             tracks_synced_at timestamptz,
             sc_last_modified timestamptz,
             sc_synced_at timestamptz NOT NULL DEFAULT now(),
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE playlist_tracks (
             playlist_urn text NOT NULL,
             position integer NOT NULL,
             sc_track_id text NOT NULL,
             PRIMARY KEY (playlist_urn, position)
         );
         CREATE INDEX playlist_tracks_track_idx ON playlist_tracks (sc_track_id);
         CREATE INDEX playlist_tracks_playlist_idx ON playlist_tracks (playlist_urn);
         CREATE TABLE sync_queue (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             user_id text NOT NULL,
             action_type text NOT NULL,
             target_urn text NOT NULL,
             generation bigint NOT NULL DEFAULT 1,
             retry_count integer NOT NULL DEFAULT 0,
             payload jsonb,
             last_error text,
             next_run_at timestamptz NOT NULL DEFAULT now(),
             failed_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now(),
             remote_attempted_generation bigint,
             remote_completed_generation bigint,
             remote_result jsonb,
             dead boolean NOT NULL DEFAULT false
         );
         CREATE UNIQUE INDEX sync_queue_target_uq
             ON sync_queue (user_id, action_type, target_urn)
             WHERE action_type <> 'comment';
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
             urn text NOT NULL UNIQUE,
             title text NOT NULL,
             title_normalized text NOT NULL,
             description text,
             genre text,
             tags text[] NOT NULL DEFAULT '{}',
             duration_ms integer NOT NULL,
             artwork_url text,
             permalink_url text,
             waveform_url text,
             language varchar(8),
             isrc text,
             metadata_artist text,
             sharing varchar(8) NOT NULL DEFAULT 'public',
             sc_created_at timestamptz,
             sc_last_modified timestamptz,
             uploader_sc_user_id text,
             release_year smallint,
             release_date date,
             storage_state text NOT NULL DEFAULT 'none',
             storage_attempts integer NOT NULL DEFAULT 0,
             duration_resolve_attempts integer NOT NULL DEFAULT 0,
             duration_resolve_retry_at timestamptz,
             is_cover boolean NOT NULL DEFAULT false,
             uploader_urn text,
             uploader_username text,
             uploader_avatar_url text,
             play_count_sc bigint,
             likes_count_sc bigint,
             reposts_count_sc bigint,
             comments_count_sc bigint,
             needs_duration_resolve boolean NOT NULL DEFAULT false,
             index_priority smallint NOT NULL DEFAULT 5,
             storage_priority smallint NOT NULL DEFAULT 5,
             sc_synced_at timestamptz NOT NULL DEFAULT now(),
             updated_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0091_catalog_metadata_observations.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0092_catalog_mutation_observation_fence.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0093_track_mutations.sql"
    ))
    .execute(pool)
    .await?;
    seed_legacy_playlist(pool).await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0077_playlist_membership_shadow.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0084_playlist_reconcile_due_index.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0085_playlist_reconcile_backoff.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0086_playlist_legacy_drain.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0094_playlist_metadata_mutations.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0102_playlist_sweep_fairness.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0108_playlist_membership_remote_apply.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(
        "CREATE TABLE oauth_apps (
             id uuid PRIMARY KEY,
             client_id text NOT NULL,
             client_secret text NOT NULL,
             active boolean NOT NULL DEFAULT true
         );
         CREATE TABLE soundcloud_connections (
             id uuid PRIMARY KEY,
             soundcloud_user_id text NOT NULL,
             oauth_app_id uuid,
             access_token text NOT NULL,
             refresh_token text NOT NULL,
             expires_at timestamptz NOT NULL,
             scope text NOT NULL,
             refresh_generation bigint NOT NULL DEFAULT 1,
             refresh_failure_count integer NOT NULL DEFAULT 0,
             refresh_lease_id uuid,
             refresh_lease_expires_at timestamptz,
             last_refresh_error_kind text,
             retry_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE background_jobs (
             id uuid PRIMARY KEY,
             kind varchar(96) NOT NULL,
             lane varchar(16) NOT NULL,
             dedup_key text,
             payload jsonb NOT NULL,
             priority smallint NOT NULL DEFAULT 0,
             generation bigint NOT NULL DEFAULT 1,
             attempts integer NOT NULL DEFAULT 0,
             max_attempts smallint NOT NULL DEFAULT 8,
             available_at timestamptz NOT NULL DEFAULT now(),
             lease_id uuid,
             lease_generation bigint,
             leased_by text,
             lease_expires_at timestamptz,
             last_error text,
             created_at timestamptz NOT NULL DEFAULT now(),
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE UNIQUE INDEX background_jobs_dedup_idx
             ON background_jobs (kind, dedup_key) WHERE dedup_key IS NOT NULL;
         CREATE TABLE oauth_app_request_cooldowns (
             oauth_app_id uuid PRIMARY KEY REFERENCES oauth_apps(id) ON DELETE CASCADE,
             retry_at timestamptz NOT NULL,
             failure_count integer NOT NULL DEFAULT 1,
             updated_at timestamptz NOT NULL DEFAULT now(),
             CONSTRAINT oauth_app_request_cooldowns_failure_count_valid
                 CHECK (failure_count > 0)
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_legacy_playlist(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO playlists (
             urn, owner_sc_user_id, track_count, desired_rev, synced_rev
         ) VALUES ('soundcloud:playlists:42', '42', 11, 1, 0)",
    )
    .execute(pool)
    .await?;
    let existing = track_ids().into_iter().take(11).collect::<Vec<_>>();
    for (position, track_id) in existing.iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_tracks (playlist_urn, position, sc_track_id)
             VALUES ('soundcloud:playlists:42', $1, $2)",
        )
        .bind(i32::try_from(position)?)
        .bind(track_id)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, urn, title, title_normalized, duration_ms
             ) VALUES ($1, 'soundcloud:tracks:' || $1, 'existing', 'existing', 1000)",
        )
        .bind(track_id)
        .execute(pool)
        .await?;
    }
    sqlx::query(
        "INSERT INTO user_owned_playlists (user_id, playlist_urn, progress, synced_at)
         VALUES ('42', 'soundcloud:playlists:42', true, now())",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_playlist_and_connection(pool: &PgPool) -> anyhow::Result<()> {
    let oauth_app_id = Uuid::from_u128(1);
    sqlx::query(
        "INSERT INTO oauth_apps (id, client_id, client_secret)
         VALUES ($1, 'client-id', 'client-secret')",
    )
    .bind(oauth_app_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
         ) VALUES (
             $1, '42', $2, 'valid-access', 'unused-refresh', now() + interval '1 day', ''
         )",
    )
    .bind(Uuid::now_v7())
    .bind(oauth_app_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn single_connection_pool(source: &PgPool) -> anyhow::Result<PgPool> {
    let database = sqlx::query_scalar::<_, String>("SELECT current_database()")
        .fetch_one(source)
        .await?;
    let mut url = Url::parse(&std::env::var("DATABASE_URL")?)?;
    url.set_path(&database);
    Ok(PgPoolOptions::new()
        .max_connections(1)
        .connect(url.as_str())
        .await?)
}

fn sync_config(api_url: Url) -> SyncQueueConfig {
    SyncQueueConfig {
        api_url: api_url.clone(),
        proxy_url: None,
        storage_url: api_url,
        storage_token: "unused".to_owned().into(),
        concurrency: 1,
        claim_batch: 1,
        lease_duration: Duration::from_secs(300),
    }
}

type MockServer = (
    Url,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
);

async fn mock_soundcloud() -> anyhow::Result<MockServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let api_url = Url::parse(&format!("http://{address}/"))?;
    let metadata = json!({
        "id": 42,
        "user": { "id": 42 },
        "track_count": 28,
        "last_modified": "2026-08-20T10:00:00Z"
    })
    .to_string();
    let collection = track_ids()
        .into_iter()
        .enumerate()
        .map(|(index, track_id)| {
            json!({
                "id": track_id,
                "urn": format!("soundcloud:tracks:{track_id}"),
                "title": format!("Track {}", index + 1),
                "duration": 120_000 + index,
                "sharing": "public",
                "user": { "id": 42, "username": "artist" }
            })
        })
        .collect::<Vec<_>>();
    let tracks = json!({
        "collection": collection,
        "next_href": null
    })
    .to_string();
    let responses = [metadata.clone(), tracks, metadata];
    let methods = Arc::new(Mutex::new(Vec::new()));
    let server_methods = Arc::clone(&methods);
    let (first_request_tx, first_request_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut first_request_tx = Some(first_request_tx);
        let mut release_rx = Some(release_rx);
        for response in responses {
            let (mut stream, _) = listener.accept().await?;
            let request = read_request(&mut stream).await?;
            let method = request
                .split_whitespace()
                .next()
                .ok_or_else(|| anyhow::anyhow!("mock received an empty request"))?;
            server_methods.lock().await.push(method.to_owned());
            if let Some(sender) = first_request_tx.take() {
                sender
                    .send(())
                    .map_err(|_| anyhow::anyhow!("request observer was dropped"))?;
                release_rx
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("mock release receiver is missing"))?
                    .await
                    .map_err(|_| anyhow::anyhow!("mock release sender was dropped"))?;
            }
            write_json_response(&mut stream, &response).await?;
        }
        Ok(())
    });
    Ok((api_url, first_request_rx, release_tx, methods, server))
}

fn track_ids() -> Vec<String> {
    (0_u64..28)
        .map(|offset| (9_007_199_254_740_000_u64 + offset).to_string())
        .collect()
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> anyhow::Result<String> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || request.len().saturating_add(read) > 16 * 1024 {
            return Err(anyhow::anyhow!("mock received an invalid request"));
        }
        request.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8(request)?)
}

async fn write_json_response(stream: &mut tokio::net::TcpStream, body: &str) -> anyhow::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

const RECONCILABLE: &str = "soundcloud:playlists:77";
const RECONCILABLE_OWNER: &str = "77";

async fn seed_reconcilable_playlist(pool: &PgPool, projection: &[&str]) -> anyhow::Result<Uuid> {
    sqlx::query(
        "INSERT INTO playlists (urn, owner_sc_user_id, track_count)
         VALUES ($1, $2, $3)",
    )
    .bind(RECONCILABLE)
    .bind(RECONCILABLE_OWNER)
    .bind(i32::try_from(projection.len())?)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_owned_playlists (user_id, playlist_urn, progress, synced_at)
         VALUES ($1, $2, true, now())",
    )
    .bind(RECONCILABLE_OWNER)
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn, next_reconcile_at)
         VALUES ($1, clock_timestamp())",
    )
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    let snapshot_id: Uuid = sqlx::query_scalar(
        "INSERT INTO playlist_remote_snapshots (playlist_urn, content_fingerprint, track_count)
         VALUES ($1, sha256($2::bytea), $3) RETURNING id",
    )
    .bind(RECONCILABLE)
    .bind(projection.join(",").into_bytes())
    .bind(i32::try_from(projection.len())?)
    .fetch_one(pool)
    .await?;
    for (position, sc_track_id) in projection.iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_remote_snapshot_tracks (snapshot_id, position, sc_track_id)
             VALUES ($1, $2, $3)",
        )
        .bind(snapshot_id)
        .bind(i32::try_from(position)?)
        .bind(sc_track_id)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
             VALUES ($1, $2, $3)",
        )
        .bind(RECONCILABLE)
        .bind(i32::try_from(position)?)
        .bind(sc_track_id)
        .execute(pool)
        .await?;
    }
    let observation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO playlist_remote_observations (
             playlist_urn, snapshot_id, authority, outcome, pagination_complete,
             all_items_identified, declared_track_count, observed_track_count
         ) VALUES ($1, $2, 'owner', 'complete', true, true, $3, $3) RETURNING id",
    )
    .bind(RECONCILABLE)
    .bind(snapshot_id)
    .bind(i32::try_from(projection.len())?)
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
    .bind(RECONCILABLE)
    .bind(observation_id)
    .bind(i32::try_from(projection.len())?)
    .execute(pool)
    .await?;
    Ok(observation_id)
}

async fn ensure_catalog(pool: &PgPool, track_ids: &[&str]) -> anyhow::Result<()> {
    for sc_track_id in track_ids {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ($1, 'soundcloud:tracks:' || $1, 'seeded', 'seeded', 1000)
             ON CONFLICT (sc_track_id) DO NOTHING",
        )
        .bind(sc_track_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn journal_operation(
    pool: &PgPool,
    observation_id: Uuid,
    sequence: i64,
    kind: &str,
    track_id: Option<&str>,
    boundary: Option<&str>,
    ordered: Option<Vec<Option<String>>>,
) -> anyhow::Result<Uuid> {
    let operation_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO playlist_membership_operations (
             operation_id, playlist_urn, sequence, actor_sc_user_id, idempotency_key,
             request_fingerprint, base_baseline_generation, base_observation_id,
             expected_projection_revision, accepted_projection_revision,
             kind, track_id, boundary, ordered_track_ids
         ) VALUES ($1, $2, $3, $4, $5, sha256($1::text::bytea), 1, $6, $3 - 1, $3, $7, $8, $9, $10)",
    )
    .bind(operation_id)
    .bind(RECONCILABLE)
    .bind(sequence)
    .bind(RECONCILABLE_OWNER)
    .bind(Uuid::now_v7())
    .bind(observation_id)
    .bind(kind)
    .bind(track_id)
    .bind(boundary)
    .bind(ordered)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE playlist_membership_state
         SET last_operation_sequence = $2,
             projection_revision = $2,
             sync_status = 'pending',
             conflict_code = NULL
         WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .bind(sequence)
    .execute(pool)
    .await?;
    Ok(operation_id)
}

fn snapshot_of(track_ids: &[&str]) -> super::model::PlaylistSnapshot {
    super::model::PlaylistSnapshot {
        playlist_id: "77".to_owned(),
        owner_id: RECONCILABLE_OWNER.to_owned(),
        track_ids: track_ids.iter().map(|value| (*value).to_owned()).collect(),
        hydrated_tracks: Vec::new(),
        track_count: track_ids.len() as i32,
        remote_last_modified: chrono::Utc::now(),
        observed_at: chrono::Utc::now(),
    }
}

async fn reduce_once(
    pool: &PgPool,
    snapshot: &super::model::PlaylistSnapshot,
) -> anyhow::Result<()> {
    reduce_with(pool, snapshot, false).await
}

async fn reduce_with(
    pool: &PgPool,
    snapshot: &super::model::PlaylistSnapshot,
    membership_remote_apply: bool,
) -> anyhow::Result<()> {
    let repository = PlaylistObserveRepository::new(pool.clone(), membership_remote_apply);
    let urn = super::urn::PlaylistUrn::parse(RECONCILABLE)?;
    let captured = repository.capture(&urn, Uuid::now_v7(), 1).await?;
    let capture = captured.capture.ok_or_else(|| anyhow::anyhow!("capture"))?;
    repository
        .persist_success(
            &capture,
            snapshot,
            catalog_ingest::Observation::begin(pool).await?,
        )
        .await?;
    Ok(())
}

async fn projection_of(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let ids = sqlx::query_scalar::<_, String>(
        "SELECT sc_track_id FROM playlist_track_projection
         WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(RECONCILABLE)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

async fn membership_of(pool: &PgPool) -> anyhow::Result<(String, Option<String>, i64, bool)> {
    let row = sqlx::query_as::<_, (String, Option<String>, i64, bool)>(
        "SELECT sync_status, conflict_code, committed_operation_sequence,
                candidate_fingerprint IS NOT NULL
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

async fn operation_of(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<(String, Option<String>, bool)> {
    let row = sqlx::query_as::<_, (String, Option<String>, bool)>(
        "SELECT outcome, conflict_code, resolved_at IS NOT NULL
         FROM playlist_membership_operations WHERE operation_id = $1",
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn membership_mutations(pool: &PgPool) -> anyhow::Result<Vec<(String, Value)>> {
    let rows = sqlx::query_as::<_, (String, Value)>(
        "SELECT user_id, payload FROM sync_queue
         WHERE action_type = 'playlist_membership' AND target_urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn applied_mark(pool: &PgPool) -> anyhow::Result<Option<(String, i64)>> {
    let row = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        "SELECT encode(remote_apply_fingerprint, 'hex'), remote_apply_generation
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_one(pool)
    .await?;
    Ok(row.0.zip(row.1))
}

async fn record_applied_mark(pool: &PgPool, tracks: &[&str]) -> anyhow::Result<()> {
    let fingerprint = super::fingerprint::membership_fingerprint(
        &tracks.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>(),
    );
    sqlx::query(
        "UPDATE playlist_membership_state
         SET remote_apply_fingerprint = $2,
             remote_apply_generation = reconcile_generation,
             remote_applied_at = clock_timestamp()
         WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .bind(fingerprint)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_rebased_playlist(pool: &PgPool) -> anyhow::Result<()> {
    ensure_catalog(pool, &["1", "2", "7", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(pool, &["1", "2"]).await?;
    journal_operation(
        pool,
        observation_id,
        1,
        "add",
        Some("9"),
        Some("back"),
        None,
    )
    .await?;
    sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ($1, 2, '9')",
    )
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_shadow_ready_candidate_is_offered_to_soundcloud_only_when_remote_apply_is_on(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_rebased_playlist(&pool).await?;

    reduce_with(&pool, &snapshot_of(&["1", "7", "2"]), false).await?;
    assert!(
        membership_mutations(&pool).await?.is_empty(),
        "the shadow path must stay shadow while the flag is off"
    );

    reduce_with(&pool, &snapshot_of(&["1", "7", "2"]), true).await?;

    let candidate = vec!["1", "7", "2", "9"];
    let fingerprint = hex_of(&super::fingerprint::membership_fingerprint(
        &candidate
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>(),
    ));
    assert_eq!(
        membership_mutations(&pool).await?,
        vec![(
            RECONCILABLE_OWNER.to_owned(),
            json!({
                "tracks": candidate,
                "fingerprint": fingerprint,
                "reconcile_generation": 2
            })
        )]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unconfirmed_apply_is_not_offered_to_soundcloud_a_second_time(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_rebased_playlist(&pool).await?;

    reduce_with(&pool, &snapshot_of(&["1", "7", "2"]), true).await?;
    record_applied_mark(&pool, &["1", "7", "2", "9"]).await?;
    sqlx::query("DELETE FROM sync_queue WHERE target_urn = $1")
        .bind(RECONCILABLE)
        .execute(&pool)
        .await?;

    reduce_with(&pool, &snapshot_of(&["1", "7", "2"]), true).await?;

    assert!(
        membership_mutations(&pool).await?.is_empty(),
        "the same candidate must not be pushed again before the next observation confirms it"
    );
    assert!(applied_mark(&pool).await?.is_some());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_observation_that_finds_the_applied_membership_drops_the_mark(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_rebased_playlist(&pool).await?;
    record_applied_mark(&pool, &["1", "7", "2", "9"]).await?;

    reduce_with(&pool, &snapshot_of(&["1", "7", "2", "9"]), true).await?;

    assert_eq!(applied_mark(&pool).await?, None);
    assert_eq!(
        membership_of(&pool).await?,
        ("clean".to_owned(), None, 1, false)
    );
    assert!(membership_mutations(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_apply_that_soundcloud_ignored_is_retried_once_the_mark_goes_stale(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_rebased_playlist(&pool).await?;
    record_applied_mark(&pool, &["1", "7", "2", "9"]).await?;
    sqlx::query(
        "UPDATE playlist_membership_state
         SET remote_applied_at = clock_timestamp() - interval '2 hours'
         WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .execute(&pool)
    .await?;

    reduce_with(&pool, &snapshot_of(&["1", "7", "2"]), true).await?;

    assert_eq!(applied_mark(&pool).await?, None);
    assert_eq!(membership_mutations(&pool).await?.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_pending_addition_rebases_onto_an_unseen_remote_addition(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "7", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(&pool, &["1", "2"]).await?;
    let operation_id = journal_operation(
        &pool,
        observation_id,
        1,
        "add",
        Some("9"),
        Some("back"),
        None,
    )
    .await?;
    sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ($1, 2, '9')",
    )
    .bind(RECONCILABLE)
    .execute(&pool)
    .await?;

    reduce_once(&pool, &snapshot_of(&["1", "7", "2"])).await?;

    assert_eq!(projection_of(&pool).await?, vec!["1", "7", "2", "9"]);
    assert_eq!(
        membership_of(&pool).await?,
        ("shadow_ready".to_owned(), None, 0, true)
    );
    assert_eq!(
        operation_of(&pool, operation_id).await?,
        ("pending".to_owned(), None, false)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_operation_the_remote_already_carries_is_committed_and_the_playlist_goes_clean(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(&pool, &["1", "2"]).await?;
    let operation_id = journal_operation(
        &pool,
        observation_id,
        1,
        "add",
        Some("9"),
        Some("back"),
        None,
    )
    .await?;

    reduce_once(&pool, &snapshot_of(&["1", "2", "9"])).await?;

    assert_eq!(projection_of(&pool).await?, vec!["1", "2", "9"]);
    assert_eq!(
        membership_of(&pool).await?,
        ("clean".to_owned(), None, 1, false)
    );
    assert_eq!(
        operation_of(&pool, operation_id).await?,
        ("committed".to_owned(), None, true)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unreplayable_operation_becomes_an_explainable_conflict(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "5"]).await?;
    let observation_id = seed_reconcilable_playlist(&pool, &["1", "2", "5"]).await?;
    let operation_id = journal_operation(
        &pool,
        observation_id,
        1,
        "move",
        Some("5"),
        Some("front"),
        None,
    )
    .await?;

    reduce_once(&pool, &snapshot_of(&["1", "2"])).await?;

    assert_eq!(
        membership_of(&pool).await?,
        (
            "conflict".to_owned(),
            Some("move_target_missing".to_owned()),
            1,
            true
        )
    );
    assert_eq!(
        operation_of(&pool, operation_id).await?,
        (
            "conflict".to_owned(),
            Some("move_target_missing".to_owned()),
            true
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_reorder_carrying_a_null_element_still_reconciles(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "3"]).await?;
    let observation_id = seed_reconcilable_playlist(&pool, &["1", "2", "3"]).await?;
    let operation_id = journal_operation(
        &pool,
        observation_id,
        1,
        "reorder",
        None,
        None,
        Some(vec![Some("3".to_owned()), None, Some("1".to_owned())]),
    )
    .await?;

    reduce_once(&pool, &snapshot_of(&["1", "2", "3"])).await?;

    assert_eq!(projection_of(&pool).await?, vec!["3", "2", "1"]);
    assert_eq!(
        operation_of(&pool, operation_id).await?,
        ("pending".to_owned(), None, false)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_membership_write_during_the_read_supersedes_the_run(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(&pool, &["1", "2"]).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);
    let urn = super::urn::PlaylistUrn::parse(RECONCILABLE)?;
    let captured = repository.capture(&urn, Uuid::now_v7(), 1).await?;
    let capture = captured.capture.ok_or_else(|| anyhow::anyhow!("capture"))?;
    journal_operation(
        &pool,
        observation_id,
        1,
        "add",
        Some("9"),
        Some("back"),
        None,
    )
    .await?;

    let result = repository
        .persist_success(
            &capture,
            &snapshot_of(&["1", "2"]),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;

    assert_eq!(result, super::repository::PersistResult::Superseded);
    assert_eq!(projection_of(&pool).await?, vec!["1", "2"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn deleting_a_playlist_finishes_inflight_observation_without_rescheduling(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "9"]).await?;
    seed_reconcilable_playlist(&pool, &["1", "2"]).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);
    let urn = super::urn::PlaylistUrn::parse(RECONCILABLE)?;
    let capture = repository
        .capture(&urn, Uuid::now_v7(), 1)
        .await?
        .capture
        .ok_or_else(|| anyhow::anyhow!("capture"))?;
    let observation = catalog_ingest::Observation::begin(&pool).await?;

    let mut transaction = pool.begin().await?;
    sqlx::query_file_scalar!(
        "../api/queries/playlists/service/lock_membership.sql",
        RECONCILABLE
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query_file_scalar!(
        "../api/queries/playlists/service/apply_delete.sql",
        RECONCILABLE,
        RECONCILABLE_OWNER
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query_file!(
        "../api/queries/playlists/service/retire_membership.sql",
        RECONCILABLE
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    assert_eq!(
        repository
            .persist_success(&capture, &snapshot_of(&["1", "2", "9"]), observation)
            .await?,
        super::repository::PersistResult::Finished
    );
    assert_eq!(
        repository
            .persist_failure(
                &capture,
                &super::repository::FailureObservation {
                    outcome: "retryable_error",
                    run_decision: "retry_wait",
                    state_status: "pending",
                    conflict_code: None,
                    retry_at: Some(chrono::Utc::now()),
                    error_kind: "upstream_unavailable".to_owned(),
                    observed_at: chrono::Utc::now(),
                },
            )
            .await?,
        super::repository::PersistResult::Finished
    );
    assert_eq!(projection_of(&pool).await?, vec!["1", "2"]);
    assert_eq!(
        repository.capture(&urn, Uuid::now_v7(), 1).await?.result,
        super::repository::CaptureResult::Finished
    );
    assert!(
        !repository
            .claim_due(16, 8, 600)
            .await?
            .contains(&RECONCILABLE.to_owned())
    );
    let state: (bool, bool, i64) = sqlx::query_as(
        "SELECT playlist.deleted_at IS NOT NULL, state.next_reconcile_at IS NULL,
                (SELECT count(*) FROM playlist_remote_observations WHERE playlist_urn = $1)
         FROM playlists AS playlist JOIN playlist_membership_state AS state
         ON state.playlist_urn = playlist.urn WHERE playlist.urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, (true, true, 1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_sweep_claims_due_playlists_and_leaves_clean_ones_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_reconcilable_playlist(&pool, &["1", "2"]).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let first = repository.claim_due(16, 8, 600).await?;
    assert!(!first.contains(&RECONCILABLE.to_owned()));

    sqlx::query(
        "UPDATE playlist_membership_state
         SET sync_status = 'pending', next_reconcile_at = clock_timestamp()
         WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .execute(&pool)
    .await?;

    assert_eq!(
        repository.claim_due(16, 8, 600).await?,
        vec![RECONCILABLE.to_owned()]
    );
    assert!(repository.claim_due(16, 8, 600).await?.is_empty());
    Ok(())
}

type ScriptedServer = (
    Url,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
);

async fn scripted_soundcloud(
    status: &'static str,
    headers: &'static str,
) -> anyhow::Result<ScriptedServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let api_url = Url::parse(&format!("http://{address}/"))?;
    let methods = Arc::new(Mutex::new(Vec::new()));
    let server_methods = Arc::clone(&methods);
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let request = read_request(&mut stream).await?;
            let method = request
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            server_methods.lock().await.push(method);
            let head = format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: 2\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(b"{}").await?;
            stream.shutdown().await?;
        }
    });
    Ok((api_url, methods, server))
}

fn observe_handler(pool: &PgPool, api_url: Url) -> anyhow::Result<PlaylistObserveHandler> {
    let sync = sync_config(api_url.clone());
    let oauth = OAuthConfig {
        token_url: api_url.join("oauth/token")?,
        bootstrap_app: None,
    };
    Ok(PlaylistObserveHandler {
        pool: pool.clone(),
        queue: JobRepository::new(pool.clone(), "playlist-observe-test".to_owned()),
        repository: PlaylistObserveRepository::new(pool.clone(), false),
        connections: ConnectionManager::new(pool.clone()),
        token_client: TokenRefreshClient::new(&oauth)?,
        reader: PlaylistReader::new(PlaylistReadClient::new(&sync)?),
        reconcile: PlaylistReconcileConfig {
            sweep_batch: 512,
            sweep_owner_share: 8,
            claim_seconds: 600,
            legacy_drain_batch: 500,
            membership_remote_apply: false,
        },
    })
}

async fn observe_state(
    pool: &PgPool,
) -> anyhow::Result<(String, Option<chrono::DateTime<chrono::Utc>>, String)> {
    Ok(
        sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>, String)>(
            "SELECT sync_status, next_reconcile_at, last_error
         FROM playlist_membership_state
         WHERE playlist_urn = 'soundcloud:playlists:42'",
        )
        .fetch_one(pool)
        .await?,
    )
}

#[sqlx::test(migrations = false)]
async fn a_rate_limited_read_defers_to_its_retry_after_and_cools_the_whole_app(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_playlist_and_connection(&pool).await?;
    let (api_url, methods, server) =
        scripted_soundcloud("429 Too Many Requests", "Retry-After: 900\r\n").await?;
    let handler = observe_handler(&pool, api_url)?;

    handler
        .observe(
            Uuid::now_v7(),
            1,
            PlaylistObservePayload {
                playlist_urn: "soundcloud:playlists:42".to_owned(),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    server.abort();

    assert_eq!(methods.lock().await.clone(), ["GET"]);
    let (status, due, error) = observe_state(&pool).await?;
    assert_eq!(status, "retry_wait");
    assert_eq!(error, "soundcloud_rate_limited");
    let due = due.ok_or_else(|| anyhow::anyhow!("failed observation left no due time"))?;
    assert!(
        due >= chrono::Utc::now() + chrono::Duration::seconds(880),
        "Retry-After was shortened by generic backoff: {due}"
    );

    let claimed = sqlx::query_scalar::<_, String>(
        "SELECT playlist_urn FROM playlist_membership_state
         WHERE sync_status <> 'clean' AND next_reconcile_at <= now()",
    )
    .fetch_all(&pool)
    .await?;
    assert!(
        claimed.is_empty(),
        "a deferred playlist was still due: {claimed:?}"
    );

    let cooldown = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT retry_at FROM oauth_app_request_cooldowns WHERE oauth_app_id = $1",
    )
    .bind(Uuid::from_u128(1))
    .fetch_one(&pool)
    .await?;
    assert!(cooldown > chrono::Utc::now());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_cooling_application_is_not_asked_again_before_its_cooldown_expires(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_playlist_and_connection(&pool).await?;
    sqlx::query(
        "INSERT INTO oauth_app_request_cooldowns (oauth_app_id, retry_at)
         VALUES ($1, now() + interval '20 minutes')",
    )
    .bind(Uuid::from_u128(1))
    .execute(&pool)
    .await?;
    let (api_url, methods, server) = scripted_soundcloud("200 OK", "").await?;
    let handler = observe_handler(&pool, api_url)?;

    handler
        .observe(
            Uuid::now_v7(),
            1,
            PlaylistObservePayload {
                playlist_urn: "soundcloud:playlists:42".to_owned(),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    server.abort();

    assert!(
        methods.lock().await.is_empty(),
        "a cooling application was asked anyway"
    );
    let (status, due, error) = observe_state(&pool).await?;
    assert_eq!(status, "retry_wait");
    assert_eq!(error, "soundcloud_app_cooling_down");
    let due = due.ok_or_else(|| anyhow::anyhow!("deferred observation left no due time"))?;
    assert!(due >= chrono::Utc::now() + chrono::Duration::seconds(1180));
    let failures = sqlx::query_scalar::<_, i32>(
        "SELECT failure_count FROM oauth_app_request_cooldowns WHERE oauth_app_id = $1",
    )
    .bind(Uuid::from_u128(1))
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        failures, 1,
        "deferring a playlist extended the app cooldown"
    );
    Ok(())
}

async fn due_in(pool: &PgPool) -> anyhow::Result<chrono::Duration> {
    let due = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT next_reconcile_at FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_one(pool)
    .await?;
    Ok(due - chrono::Utc::now())
}

async fn queued_track_refreshes(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT dedup_key FROM background_jobs
         WHERE kind = 'catalog.refresh' ORDER BY dedup_key",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn an_unhydrated_remote_addition_schedules_its_own_catalog_refresh_and_retries_soon(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(&pool, &["1", "2"]).await?;
    let operation_id = journal_operation(
        &pool,
        observation_id,
        1,
        "add",
        Some("9"),
        Some("back"),
        None,
    )
    .await?;
    sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ($1, 2, '9')",
    )
    .bind(RECONCILABLE)
    .execute(&pool)
    .await?;

    reduce_once(&pool, &snapshot_of(&["1", "7", "2"])).await?;

    let (status, conflict, committed, _) = membership_of(&pool).await?;
    assert_eq!(status, "conflict");
    assert_eq!(conflict.as_deref(), Some("catalog_incomplete"));
    assert_eq!(committed, 0);
    assert_eq!(
        operation_of(&pool, operation_id).await?,
        ("pending".to_owned(), None, false),
        "the local addition must keep waiting, not be dropped"
    );
    assert_eq!(queued_track_refreshes(&pool).await?, ["track:7:public"]);
    let due = due_in(&pool).await?;
    assert!(
        due < chrono::Duration::minutes(5),
        "an incomplete catalog parked the playlist for {due}"
    );

    reduce_once(&pool, &snapshot_of(&["1", "7", "2"])).await?;
    let backed_off = due_in(&pool).await?;
    assert!(
        backed_off > due && backed_off < chrono::Duration::hours(6),
        "a permanently unhydratable track must back off, not hot-loop: {backed_off}"
    );

    ensure_catalog(&pool, &["7"]).await?;
    reduce_once(&pool, &snapshot_of(&["1", "7", "2"])).await?;

    assert_eq!(projection_of(&pool).await?, vec!["1", "7", "2", "9"]);
    assert!(
        due_in(&pool).await? < chrono::Duration::hours(2),
        "a recovered playlist must not stay parked on the stuck backoff"
    );
    assert_eq!(
        membership_of(&pool).await?,
        ("shadow_ready".to_owned(), None, 0, true)
    );
    assert_eq!(
        operation_of(&pool, operation_id).await?,
        ("pending".to_owned(), None, false)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_missing_connection_parks_the_intent_and_keeps_the_app_session(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_playlist_and_connection(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO playlists (urn, owner_sc_user_id, track_count)
         VALUES ('soundcloud:playlists:404', '999', 0);
         INSERT INTO playlist_membership_state (playlist_urn, next_reconcile_at)
         VALUES ('soundcloud:playlists:404', clock_timestamp());",
    )
    .execute(&pool)
    .await?;
    let (api_url, methods, server) = scripted_soundcloud("200 OK", "").await?;
    let handler = observe_handler(&pool, api_url)?;

    handler
        .observe(
            Uuid::now_v7(),
            1,
            PlaylistObservePayload {
                playlist_urn: "soundcloud:playlists:404".to_owned(),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    server.abort();

    assert!(
        methods.lock().await.is_empty(),
        "a playlist without a connection still called SoundCloud"
    );
    let (status, due, error) =
        sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>, String)>(
            "SELECT sync_status, next_reconcile_at, last_error
         FROM playlist_membership_state WHERE playlist_urn = 'soundcloud:playlists:404'",
        )
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "auth_required");
    assert_eq!(error, "soundcloud_reauthorization_required");
    assert!(
        due.is_some_and(|due| due > chrono::Utc::now()),
        "a re-linkable intent must keep waiting, not be abandoned"
    );
    let connections: i64 = sqlx::query_scalar("SELECT count(*) FROM soundcloud_connections")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        connections, 1,
        "a failed observation removed an app session"
    );
    let apps: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_apps")
        .fetch_one(&pool)
        .await?;
    assert_eq!(apps, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_playlist_without_local_operations_follows_the_remote(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    ensure_catalog(&pool, &["1", "2", "7"]).await?;
    seed_reconcilable_playlist(&pool, &["1", "2"]).await?;

    reduce_once(&pool, &snapshot_of(&["1", "7", "2"])).await?;

    assert_eq!(projection_of(&pool).await?, vec!["1", "7", "2"]);
    let (status, conflict, committed, fingerprint) = membership_of(&pool).await?;
    assert_eq!(status, "clean");
    assert_eq!(conflict, None);
    assert_eq!(committed, 0);
    assert!(
        !fingerprint,
        "a clean playlist keeps no candidate fingerprint"
    );
    let due = due_in(&pool).await?;
    assert!(
        due > chrono::Duration::minutes(4) && due < chrono::Duration::minutes(6),
        "a clean playlist should be revisited on the clean cadence: {due}"
    );

    reduce_once(&pool, &snapshot_of(&["7"])).await?;
    assert_eq!(projection_of(&pool).await?, vec!["7"]);
    assert_eq!(membership_of(&pool).await?.0, "clean");
    Ok(())
}

async fn seed_owned_playlists(pool: &PgPool, owner: &str, count: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO playlists (urn, owner_sc_user_id, track_count)
         SELECT 'soundcloud:playlists:' || $1 || '-' || n, $1, 0
         FROM generate_series(1, $2) n",
    )
    .bind(owner)
    .bind(count)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state
             (playlist_urn, owner_sc_user_id, sync_status, next_reconcile_at)
         SELECT 'soundcloud:playlists:' || $1 || '-' || n, $1, 'pending', clock_timestamp()
         FROM generate_series(1, $2) n",
    )
    .bind(owner)
    .bind(count)
    .execute(pool)
    .await?;
    Ok(())
}

fn owner_of(playlist_urn: &str) -> String {
    playlist_urn
        .trim_start_matches("soundcloud:playlists:")
        .split('-')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[sqlx::test(migrations = false)]
async fn one_owner_with_a_deep_backlog_cannot_take_the_whole_sweep(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_owned_playlists(&pool, "100", 50).await?;
    seed_owned_playlists(&pool, "200", 1).await?;
    seed_owned_playlists(&pool, "300", 1).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let claimed = repository.claim_due(16, 4, 600).await?;
    let owners: std::collections::BTreeSet<String> =
        claimed.iter().map(|urn| owner_of(urn)).collect();
    let greedy = claimed.iter().filter(|urn| owner_of(urn) == "100").count();

    assert_eq!(
        greedy, 4,
        "the deep-backlog owner took {greedy} of a 4-per-owner share"
    );
    assert_eq!(
        owners,
        ["100", "200", "300", "42"]
            .into_iter()
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>(),
        "every waiting owner must be served in the same sweep, including the legacy fixture"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_sweep_rotates_owners_instead_of_restarting_from_the_first(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for owner in ["100", "200", "300", "400"] {
        seed_owned_playlists(&pool, owner, 4).await?;
    }
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let mut sweeps = Vec::new();
    for _ in 0..3 {
        let claimed = repository.claim_due(4, 2, 600).await?;
        sweeps.push(
            claimed
                .iter()
                .map(|urn| owner_of(urn))
                .collect::<std::collections::BTreeSet<String>>(),
        );
    }

    let served: std::collections::BTreeSet<String> = sweeps.iter().flatten().cloned().collect();
    assert_eq!(
        served,
        ["100", "200", "300", "400", "42"]
            .into_iter()
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>(),
        "a full rotation must reach every waiting owner: {sweeps:?}"
    );
    assert!(
        sweeps[0].is_disjoint(&sweeps[1]) && sweeps[1].is_disjoint(&sweeps[2]),
        "consecutive sweeps must not serve the same owner again: {sweeps:?}"
    );

    let wrapped = repository.claim_due(4, 2, 600).await?;
    let wrapped_owners: std::collections::BTreeSet<String> =
        wrapped.iter().map(|urn| owner_of(urn)).collect();
    assert_eq!(
        wrapped_owners, sweeps[0],
        "the rotation must wrap back to the first owners"
    );
    Ok(())
}
