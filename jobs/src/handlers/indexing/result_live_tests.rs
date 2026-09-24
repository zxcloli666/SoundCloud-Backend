use backend_contracts::vector_store::{TRACKS_CLAP, TRACKS_MERT};
use chrono::{TimeZone, Utc};
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{DeletePointsBuilder, GetPointsBuilder, PointId, PointsIdsList};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::QdrantConfig;

use super::*;

const CORRELATED_POINT_ID: u64 = 991_000_042;
const STALE_POINT_ID: u64 = 991_000_043;
const REDELIVERED_POINT_ID: u64 = 991_000_044;
const SETTLED_POINT_ID: u64 = 991_000_045;

fn track_id(point: u64) -> String {
    point.to_string()
}

fn grpc_url() -> String {
    std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned())
}

fn provisioner() -> QdrantProvisioner {
    QdrantProvisioner::connect(&QdrantConfig {
        grpc_url: grpc_url(),
        api_key: String::new().into(),
    })
    .expect("Qdrant client connects")
}

fn raw_client() -> Qdrant {
    Qdrant::from_url(&grpc_url())
        .skip_compatibility_check()
        .build()
        .expect("raw Qdrant client connects")
}

async fn forget_point(client: &Qdrant, point: u64) -> anyhow::Result<()> {
    for collection in [TRACKS_MERT, TRACKS_CLAP] {
        client
            .delete_points(
                DeletePointsBuilder::new(collection)
                    .points(PointsIdsList {
                        ids: vec![PointId::from(point)],
                    })
                    .wait(true),
            )
            .await?;
    }
    Ok(())
}

async fn stored_generation(
    client: &Qdrant,
    collection: &str,
    point: u64,
) -> anyhow::Result<Option<i64>> {
    let response = client
        .get_points(
            GetPointsBuilder::new(collection, vec![PointId::from(point)]).with_payload(true),
        )
        .await?;
    let Some(point) = response.result.into_iter().next() else {
        return Ok(None);
    };
    Ok(point
        .payload
        .get("upload_generation")
        .and_then(|value| value.as_integer()))
}

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL UNIQUE,
             storage_state varchar(16) NOT NULL,
             index_state varchar(16) NOT NULL,
             needs_duration_resolve boolean NOT NULL DEFAULT false,
             indexed_at timestamptz,
             index_attempts smallint NOT NULL DEFAULT 0,
             audio_fingerprint text,
             canonical_track_id uuid,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE storage_event_state (
             sc_track_id text PRIMARY KEY,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             uploaded_generation bigint NOT NULL,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE pipeline_event_receipts (
             consumer varchar(96) NOT NULL,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             processed_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (consumer, stream, stream_sequence, event_published_at)
         );",
    )
    .execute(pool)
    .await?;
    super::super::test_schema::install_audio_index_wire_state(pool).await
}

async fn seed_dispatched(pool: &PgPool, point: u64, generation: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             id, sc_track_id, storage_state, index_state, needs_duration_resolve
         ) VALUES ($1, $2, 'ok', 'pending', false)",
    )
    .bind(Uuid::now_v7())
    .bind(track_id(point))
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ($1, 'STORAGE_EVENTS', 1, to_timestamp(90), $2)",
    )
    .bind(track_id(point))
    .bind(generation)
    .execute(pool)
    .await?;
    let attempt = sqlx::query_file_scalar!(
        "queries/indexing/storage/dispatch_audio.sql",
        track_id(point),
        generation
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(attempt, Some(1));
    Ok(())
}

fn delivery(sequence: u64) -> DeliveryContext {
    DeliveryContext {
        consumer: "backend-done-index-audio".to_owned(),
        stream: "PIPELINE_DONE".to_owned(),
        stream_sequence: sequence,
        delivery_attempt: 1,
        published_at: Utc.timestamp_opt(200 + sequence as i64, 0).unwrap(),
    }
}

fn result(point: u64, generation: i64) -> AudioIndexResult {
    AudioIndexResult {
        sc_track_id: track_id(point),
        upload_generation: generation,
        attempt: 1,
        status: WorkerStatus::Ok,
        reason: None,
        detail: None,
        producer: backend_contracts::pipeline::Producer {
            worker_id: "gpu-main".to_owned(),
            build: "test".to_owned(),
            models: std::collections::BTreeMap::new(),
            sync_version: None,
        },
        mert: Some(vec![0.25; TRACKS_MERT_DIMENSIONS as usize]),
        clap: Some(vec![0.5; TRACKS_CLAP_DIMENSIONS as usize]),
        fingerprint: None,
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant instance"]
async fn a_correlated_result_tags_both_collections_with_its_generation(
    pool: PgPool,
) -> anyhow::Result<()> {
    let qdrant = provisioner();
    qdrant.provision().await?;
    let client = raw_client();
    forget_point(&client, CORRELATED_POINT_ID).await?;
    install_schema(&pool).await?;
    seed_dispatched(&pool, CORRELATED_POINT_ID, 3).await?;

    AudioIndexResultHandler::new(pool.clone(), qdrant)
        .finish(result(CORRELATED_POINT_ID, 3), delivery(1))
        .await?;

    let state = sqlx::query_as::<_, (String, String, Option<i64>)>(
        "SELECT track.index_state, wire.status, wire.upload_generation
         FROM tracks AS track
         JOIN audio_index_wire_state AS wire USING (sc_track_id)
         WHERE track.sc_track_id = $1",
    )
    .bind(track_id(CORRELATED_POINT_ID))
    .fetch_one(&pool)
    .await?;
    let mert = stored_generation(&client, TRACKS_MERT, CORRELATED_POINT_ID).await?;
    let clap = stored_generation(&client, TRACKS_CLAP, CORRELATED_POINT_ID).await?;
    forget_point(&client, CORRELATED_POINT_ID).await?;

    assert_eq!(state, ("indexed".to_owned(), "done".to_owned(), Some(3)));
    assert_eq!(mert, Some(3));
    assert_eq!(clap, Some(3));
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant instance"]
async fn a_stale_generation_result_never_reaches_the_vector_store(
    pool: PgPool,
) -> anyhow::Result<()> {
    let qdrant = provisioner();
    qdrant.provision().await?;
    let client = raw_client();
    forget_point(&client, STALE_POINT_ID).await?;
    install_schema(&pool).await?;
    seed_dispatched(&pool, STALE_POINT_ID, 2).await?;

    AudioIndexResultHandler::new(pool.clone(), qdrant)
        .finish(result(STALE_POINT_ID, 1), delivery(1))
        .await?;

    let state = sqlx::query_as::<_, (String, String, Option<i64>)>(
        "SELECT track.index_state, wire.status, wire.upload_generation
         FROM tracks AS track
         JOIN audio_index_wire_state AS wire USING (sc_track_id)
         WHERE track.sc_track_id = $1",
    )
    .bind(track_id(STALE_POINT_ID))
    .fetch_one(&pool)
    .await?;
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    let mert = stored_generation(&client, TRACKS_MERT, STALE_POINT_ID).await?;
    forget_point(&client, STALE_POINT_ID).await?;

    assert_eq!(state, ("pending".to_owned(), "pending".to_owned(), Some(2)));
    assert_eq!(receipts, 1);
    assert_eq!(mert, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant instance"]
async fn a_redelivered_result_rewrites_the_same_point_without_a_second_commit(
    pool: PgPool,
) -> anyhow::Result<()> {
    let qdrant = provisioner();
    qdrant.provision().await?;
    let client = raw_client();
    forget_point(&client, REDELIVERED_POINT_ID).await?;
    install_schema(&pool).await?;
    seed_dispatched(&pool, REDELIVERED_POINT_ID, 5).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), qdrant);

    handler
        .finish(result(REDELIVERED_POINT_ID, 5), delivery(1))
        .await?;
    handler
        .finish(result(REDELIVERED_POINT_ID, 5), delivery(1))
        .await?;

    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    let generation = stored_generation(&client, TRACKS_MERT, REDELIVERED_POINT_ID).await?;
    let index_state: String =
        sqlx::query_scalar("SELECT index_state FROM tracks WHERE sc_track_id = $1")
            .bind(track_id(REDELIVERED_POINT_ID))
            .fetch_one(&pool)
            .await?;
    forget_point(&client, REDELIVERED_POINT_ID).await?;

    assert_eq!(receipts, 1);
    assert_eq!(generation, Some(5));
    assert_eq!(index_state, "indexed");
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant instance"]
async fn a_result_without_vectors_settles_the_generation_and_writes_no_point(
    pool: PgPool,
) -> anyhow::Result<()> {
    let qdrant = provisioner();
    qdrant.provision().await?;
    let client = raw_client();
    forget_point(&client, SETTLED_POINT_ID).await?;
    install_schema(&pool).await?;
    seed_dispatched(&pool, SETTLED_POINT_ID, 4).await?;

    AudioIndexResultHandler::new(pool.clone(), qdrant)
        .finish(
            AudioIndexResult {
                status: WorkerStatus::Empty,
                reason: Some(WorkerReason::SilentAudio),
                mert: None,
                clap: None,
                ..result(SETTLED_POINT_ID, 4)
            },
            delivery(1),
        )
        .await?;

    let state = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT track.index_state, wire.status, wire.outcome_reason
         FROM tracks AS track
         JOIN audio_index_wire_state AS wire USING (sc_track_id)
         WHERE track.sc_track_id = $1",
    )
    .bind(track_id(SETTLED_POINT_ID))
    .fetch_one(&pool)
    .await?;
    let mert = stored_generation(&client, TRACKS_MERT, SETTLED_POINT_ID).await?;
    let clap = stored_generation(&client, TRACKS_CLAP, SETTLED_POINT_ID).await?;
    forget_point(&client, SETTLED_POINT_ID).await?;

    assert_eq!(
        state,
        (
            "failed".to_owned(),
            "terminal".to_owned(),
            Some("silent_audio".to_owned())
        )
    );
    assert_eq!(mert, None);
    assert_eq!(clap, None);
    Ok(())
}
