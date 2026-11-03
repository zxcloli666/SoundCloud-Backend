use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{StatusCode, Uri};
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::S3VerifierService;

struct Storage {
    url: String,
    heads: Arc<AtomicUsize>,
}

async fn storage_holding(present: &'static str) -> anyhow::Result<Storage> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let heads = Arc::new(AtomicUsize::new(0));
    let counted = heads.clone();
    let app = axum::Router::new().fallback(move |uri: Uri| {
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            if uri.path().ends_with(present) {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        }
    });
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Ok(Storage { url, heads })
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, storage_state, storage_quality) VALUES
         ('7001', 'soundcloud:tracks:7001', 'Kept', 'kept', 1000, 'public', 'pending', NULL),
         ('7002', 'soundcloud:tracks:7002', 'Lost', 'lost', 1000, 'public', 'ok', 'hq'),
         ('7003', 'soundcloud:tracks:7003', 'Long', 'long', 1000, 'public', 'too_long', NULL)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

type Stored = (
    String,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

async fn stored(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<Stored> {
    Ok(sqlx::query_as(
        "SELECT storage_state, storage_quality, s3_verified_at, s3_missing_at
         FROM tracks WHERE sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?)
}

fn ids() -> Vec<String> {
    ["7001", "7002", "7003"].map(str::to_owned).to_vec()
}

fn lost() -> HashSet<String> {
    HashSet::from(["7002".to_owned(), "7003".to_owned()])
}

#[sqlx::test(migrations = "./migrations")]
async fn a_writing_node_stores_both_verdicts_through_the_kind(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let storage = storage_holding("soundcloud_tracks_7001.m4a").await?;
    let verifier = S3VerifierService::new(wreq::Client::new(), storage.url, pool.clone(), true);

    let missing = verifier.find_missing(&ids()).await?;

    assert_eq!(missing, lost());
    let (state, _, verified, gone) = stored(&pool, "7001").await?;
    assert_eq!(state, "ok");
    assert!(verified.is_some() && gone.is_none());
    let (state, quality, verified, gone) = stored(&pool, "7002").await?;
    assert_eq!((state.as_str(), quality), ("pending", None));
    assert!(verified.is_none() && gone.is_some());
    let (state, _, verified, gone) = stored(&pool, "7003").await?;
    assert_eq!(state, "too_long");
    assert!(
        verified.is_none() && gone.is_none(),
        "a too long track is never marked"
    );

    let probed = storage.heads.load(Ordering::SeqCst);
    let again = verifier.find_missing(&ids()).await?;
    assert_eq!(again, lost());
    assert_eq!(
        storage.heads.load(Ordering::SeqCst),
        probed,
        "nothing is probed again, the too long track the kind never marks is answered from the process"
    );
    let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM cmd_outbox")
        .fetch_one(&pool)
        .await?;
    assert_eq!(commands, 0, "a legacy node applies the kind in place");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_node_without_api_background_keeps_its_verdicts_in_process(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let before = [
        stored(&pool, "7001").await?,
        stored(&pool, "7002").await?,
        stored(&pool, "7003").await?,
    ];
    let storage = storage_holding("soundcloud_tracks_7001.m4a").await?;
    let verifier = S3VerifierService::new(wreq::Client::new(), storage.url, pool.clone(), false);

    let missing = verifier.find_missing(&ids()).await?;
    assert_eq!(missing, lost());
    let after = [
        stored(&pool, "7001").await?,
        stored(&pool, "7002").await?,
        stored(&pool, "7003").await?,
    ];
    assert_eq!(before, after, "the node writes no verdict");

    let probed = storage.heads.load(Ordering::SeqCst);
    let again = verifier.find_missing(&ids()).await?;
    assert_eq!(again, lost());
    assert_eq!(
        storage.heads.load(Ordering::SeqCst),
        probed,
        "both verdicts are answered from the process"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_writing_api_node_in_split_sends_its_verdicts_to_core_and_leaves_its_tracks_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    sqlx::query("SELECT count(*) FROM public.scd_configure('main', 'api', 'split')")
        .execute(&pool)
        .await?;
    let api = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_with((*pool.connect_options()).clone())
        .await?;
    let before = [
        stored(&api, "7001").await?,
        stored(&api, "7002").await?,
        stored(&api, "7003").await?,
    ];
    let storage = storage_holding("soundcloud_tracks_7001.m4a").await?;
    let verifier = S3VerifierService::new(wreq::Client::new(), storage.url, api.clone(), true);

    let missing = verifier.find_missing(&ids()).await?;
    assert_eq!(missing, lost());
    let after = [
        stored(&api, "7001").await?,
        stored(&api, "7002").await?,
        stored(&api, "7003").await?,
    ];
    assert_eq!(
        before, after,
        "the api node never writes or locks a verdict row"
    );
    let shipped: (String, i32) =
        sqlx::query_as("SELECT kind, jsonb_array_length(touched) FROM cmd_outbox")
            .fetch_one(&api)
            .await?;
    assert_eq!(shipped, ("track.s3_state".to_owned(), 0));

    let probed = storage.heads.load(Ordering::SeqCst);
    let again = verifier.find_missing(&ids()).await?;
    assert_eq!(again, lost());
    assert_eq!(
        storage.heads.load(Ordering::SeqCst),
        probed,
        "until core's verdicts arrive the process answers, nothing is probed twice"
    );
    api.close().await;
    Ok(())
}
