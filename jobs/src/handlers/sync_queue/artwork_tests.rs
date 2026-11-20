use std::sync::{Arc, Mutex, PoisonError};

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};
use serde_json::json;
use sqlx::PgPool;

use crate::config::ObjectsConfig;
use crate::objects::Objects;

use super::track_tests::connect_owner;
use super::unclear_create_tests::{handler, run};

const PLAYLIST: &str = "soundcloud:playlists:42";
const KEY: &str = "42/0193.jpg";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0 staged cover \xff\xd9";
const ARTWORK_URL: &str = "https://i1.sndcdn.com/artworks-new-large.jpg";

#[derive(Clone, Default)]
struct FakePlaylists {
    calls: Arc<Mutex<Vec<Sent>>>,
    status: Arc<Mutex<Option<StatusCode>>>,
}

#[derive(Clone, Debug)]
struct Sent {
    target: String,
    authorization: String,
    content_type: String,
    body: Vec<u8>,
}

impl FakePlaylists {
    async fn serve(&self) -> anyhow::Result<std::net::SocketAddr> {
        let app = Router::new()
            .route("/playlists/{urn}", put(update))
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(address)
    }

    fn answer_with(&self, status: StatusCode) {
        *lock(&self.status) = Some(status);
    }

    fn sent(&self) -> Vec<Sent> {
        lock(&self.calls).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn update(
    State(fake): State<FakePlaylists>,
    Path(urn): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let text = |name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    lock(&fake.calls).push(Sent {
        target: urn.clone(),
        authorization: text(header::AUTHORIZATION),
        content_type: text(header::CONTENT_TYPE),
        body: body.to_vec(),
    });
    match *lock(&fake.status) {
        Some(status) => status.into_response(),
        None => Json(json!({"urn": urn, "artwork_url": ARTWORK_URL, "tracks": []})).into_response(),
    }
}

fn store_config() -> anyhow::Result<ObjectsConfig> {
    let env = |name: &str| {
        std::env::var(name)
            .map_err(|_| anyhow::anyhow!("{name} is not set: this test needs a live SeaweedFS"))
    };
    Ok(ObjectsConfig {
        endpoint: env("CHAT_TEST_S3_ENDPOINT")?.parse()?,
        region: "us-east-1".to_owned(),
        bucket: format!(
            "artwork-live-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        ),
        access_key_id: env("CHAT_TEST_S3_ACCESS_KEY")?.into(),
        secret_access_key: env("CHAT_TEST_S3_SECRET_KEY")?.into(),
        path_style: true,
    })
}

fn admin(config: &ObjectsConfig) -> aws_sdk_s3::Client {
    aws_sdk_s3::Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(config.endpoint.as_str())
            .credentials_provider(Credentials::new(
                config.access_key_id.expose(),
                config.secret_access_key.expose(),
                None,
                None,
                "artwork-live",
            ))
            .force_path_style(true)
            .build(),
    )
}

async fn stage(config: &ObjectsConfig) -> anyhow::Result<()> {
    let admin = admin(config);
    admin.create_bucket().bucket(&config.bucket).send().await?;
    admin
        .put_object()
        .bucket(&config.bucket)
        .key(KEY)
        .body(ByteStream::from(JPEG.to_vec()))
        .send()
        .await?;
    Ok(())
}

async fn queue_artwork(pool: &PgPool) -> anyhow::Result<()> {
    crate::db::migrations::run_core(pool, None).await?;
    connect_owner(pool).await?;
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "17",
        "playlist_artwork",
        PLAYLIST,
        &json!({"key": KEY}),
        false
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn queued(pool: &PgPool) -> anyhow::Result<Option<(i32, bool, Option<String>)>> {
    Ok(sqlx::query_as(
        "SELECT retry_count, dead, last_error FROM sync_queue WHERE action_type = 'playlist_artwork'",
    )
    .fetch_optional(pool)
    .await?)
}

async fn refreshes(pool: &PgPool) -> anyhow::Result<Vec<serde_json::Value>> {
    Ok(
        sqlx::query_scalar("SELECT payload FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(pool)
            .await?,
    )
}

async fn still_staged(config: &ObjectsConfig) -> anyhow::Result<bool> {
    Ok(Objects::new(config).get_bytes(KEY).await?.is_some())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_artwork_write_waits_while_no_store_is_configured(pool: PgPool) -> anyhow::Result<()> {
    queue_artwork(&pool).await?;
    let fake = FakePlaylists::default();
    let handler = handler(&pool, fake.serve().await?)?;

    run(&pool, &handler).await?;

    assert!(fake.sent().is_empty());
    let (retries, dead, error) = queued(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("row is gone"))?;
    assert_eq!((retries, dead), (0, false));
    assert!(error.is_some_and(|error| error.contains("not configured")));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
#[ignore = "needs a live SeaweedFS"]
async fn a_staged_artwork_is_sent_as_the_playlist_artwork_file_s3_live(
    pool: PgPool,
) -> anyhow::Result<()> {
    let config = store_config()?;
    stage(&config).await?;
    queue_artwork(&pool).await?;
    let fake = FakePlaylists::default();
    let mut handler = handler(&pool, fake.serve().await?)?;
    handler.artwork = Some(Objects::new(&config));

    run(&pool, &handler).await?;

    let sent = fake.sent();
    assert_eq!(sent.len(), 1);
    let call = &sent[0];
    assert_eq!(call.target, PLAYLIST);
    assert_eq!(call.authorization, "OAuth access");
    assert!(
        call.content_type
            .starts_with("multipart/form-data; boundary=")
    );
    assert!(contains(
        &call.body,
        b"Content-Disposition: form-data; name=\"playlist[artwork_data]\"; filename=\"artwork.jpg\""
    ));
    assert!(contains(&call.body, b"Content-Type: image/jpeg"));
    assert!(contains(&call.body, JPEG));
    assert_eq!(queued(&pool).await?, None);
    let refreshes = refreshes(&pool).await?;
    assert_eq!(refreshes.len(), 1);
    assert!(refreshes[0].to_string().contains("\"sc_id\":\"42\""));
    assert!(!still_staged(&config).await?);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
#[ignore = "needs a live SeaweedFS"]
async fn an_image_soundcloud_refuses_closes_the_write_without_a_retry_s3_live(
    pool: PgPool,
) -> anyhow::Result<()> {
    let config = store_config()?;
    stage(&config).await?;
    queue_artwork(&pool).await?;
    let fake = FakePlaylists::default();
    fake.answer_with(StatusCode::UNPROCESSABLE_ENTITY);
    let mut handler = handler(&pool, fake.serve().await?)?;
    handler.artwork = Some(Objects::new(&config));

    run(&pool, &handler).await?;

    assert_eq!(fake.sent().len(), 1);
    assert_eq!(queued(&pool).await?, None);
    assert!(refreshes(&pool).await?.is_empty());
    Ok(())
}
