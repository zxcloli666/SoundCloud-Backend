use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::config::{OAuthConfig, SyncQueueConfig};
use crate::handlers::star_priority::Turns;

use super::SyncQueueHandler;
use super::client::{SoundCloudClient, TokenRefreshClient};
use super::connection::ConnectionManager;
use super::repository::SyncQueueRepository;
use super::storage::TrackStorage;
use super::write_pause::WritePauses;

const CREATED: &str = "soundcloud:playlists:900";

enum Reply {
    Status(StatusCode),
    Body(StatusCode, String),
    RenewToken,
}

#[derive(Clone)]
struct FakeSoundCloud {
    pool: PgPool,
    calls: Arc<Mutex<Vec<String>>>,
    creates: Arc<Mutex<VecDeque<Reply>>>,
    listings: Arc<Mutex<VecDeque<Reply>>>,
}

impl FakeSoundCloud {
    fn new(pool: &PgPool) -> Self {
        Self {
            pool: pool.clone(),
            calls: Arc::default(),
            creates: Arc::default(),
            listings: Arc::default(),
        }
    }

    fn create_replies(&self, replies: impl IntoIterator<Item = Reply>) {
        lock(&self.creates).extend(replies);
    }

    fn listing_replies(&self, replies: impl IntoIterator<Item = Reply>) {
        lock(&self.listings).extend(replies);
    }

    fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }

    fn methods(&self) -> Vec<String> {
        self.calls()
            .iter()
            .filter_map(|call| call.split(' ').next().map(str::to_owned))
            .collect()
    }

    async fn serve(&self) -> anyhow::Result<std::net::SocketAddr> {
        let app = Router::new()
            .route("/playlists", post(create))
            .route("/me/playlists", get(listing))
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(address)
    }

    fn record(&self, method: &str, uri: &OriginalUri, headers: &HeaderMap) {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        lock(&self.calls).push(format!("{method} {} {token}", uri.0));
    }

    async fn answer(&self, reply: Option<Reply>, fallback: Response) -> Response {
        match reply {
            None => fallback,
            Some(Reply::Status(status)) => status.into_response(),
            Some(Reply::Body(status, body)) => {
                (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
            }
            Some(Reply::RenewToken) => {
                let renewed = sqlx::query(
                    "UPDATE soundcloud_connections SET access_token = 'fresh', updated_at = now()",
                )
                .execute(&self.pool)
                .await;
                match renewed {
                    Ok(_) => StatusCode::UNAUTHORIZED.into_response(),
                    Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                }
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn create(
    State(fake): State<FakeSoundCloud>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response {
    fake.record("POST", &uri, &headers);
    let reply = lock(&fake.creates).pop_front();
    fake.answer(reply, StatusCode::INTERNAL_SERVER_ERROR.into_response())
        .await
}

async fn listing(
    State(fake): State<FakeSoundCloud>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response {
    fake.record("GET", &uri, &headers);
    let reply = lock(&fake.listings).pop_front();
    fake.answer(reply, Json(json!({"collection": []})).into_response())
        .await
}

fn created_playlist() -> Value {
    json!({
        "urn": CREATED,
        "title": "Mix",
        "created_at": chrono::Utc::now().to_rfc3339(),
        "user": {"id": 17, "urn": "soundcloud:users:17", "username": "me"},
        "tracks": [{"id": 5}]
    })
}

fn created() -> Reply {
    Reply::Body(StatusCode::CREATED, created_playlist().to_string())
}

fn listed() -> Reply {
    Reply::Body(
        StatusCode::OK,
        json!({"collection": [created_playlist()], "next_href": null}).to_string(),
    )
}

pub(super) fn handler(
    pool: &PgPool,
    address: std::net::SocketAddr,
) -> anyhow::Result<SyncQueueHandler> {
    let config = SyncQueueConfig {
        api_url: format!("http://{address}/").parse()?,
        proxy_url: None,
        storage_url: "http://127.0.0.1:1/".parse()?,
        storage_token: String::new().into(),
        concurrency: 1,
        claim_batch: 1,
        lease_duration: Duration::from_secs(60),
        like_interval: Duration::from_secs(5),
    };
    let oauth = OAuthConfig {
        token_url: format!("http://{address}/oauth2/token").parse()?,
        bootstrap_app: None,
    };
    Ok(SyncQueueHandler {
        repository: Arc::new(SyncQueueRepository::new(
            pool.clone(),
            config.lease_duration,
        )),
        connections: Arc::new(ConnectionManager::new(pool.clone())),
        client: Arc::new(SoundCloudClient::new(&config)?),
        token_client: Arc::new(TokenRefreshClient::new(&oauth)?),
        storage: TrackStorage::new(&config)?,
        write_pauses: WritePauses::default(),
        account_pauses: WritePauses::default(),
        like_interval: config.like_interval,
        pauses_restored: tokio::sync::OnceCell::new(),
        concurrency: 1,
        claim_batch: 1,
        turns: Turns::default(),
    })
}

async fn setup(pool: &PgPool) -> anyhow::Result<(FakeSoundCloud, SyncQueueHandler)> {
    crate::db::migrations::run_core(pool, None).await?;
    super::track_tests::connect_owner(pool).await?;
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "17",
        "playlist_create",
        "new:7d4c4c56-8c0f-4c2e-9a37-1b0e8a4f3e21",
        &json!({"playlist": {"title": "Mix", "sharing": "private"}}),
        false
    )
    .execute(pool)
    .await?;
    let fake = FakeSoundCloud::new(pool);
    let address = fake.serve().await?;
    let handler = handler(pool, address)?;
    Ok((fake, handler))
}

pub(super) async fn run(pool: &PgPool, handler: &SyncQueueHandler) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_queue SET next_run_at = now()")
        .execute(pool)
        .await?;
    handler.repository.adopt().await?;
    let mutation = handler
        .repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("the write was not claimed"))?;
    handler.execute(mutation).await
}

async fn row(pool: &PgPool) -> anyhow::Result<Option<(i32, bool, bool)>> {
    Ok(sqlx::query_as(
        "SELECT retry_count, dead, remote_attempted_generation IS NOT DISTINCT FROM generation
         FROM sync_queue",
    )
    .fetch_optional(pool)
    .await?)
}

async fn owned(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_owned_playlists WHERE playlist_urn = $1)",
    )
    .bind(CREATED)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn an_unclear_create_found_on_soundcloud_is_finished_without_a_second_post(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    fake.create_replies([Reply::Status(StatusCode::BAD_GATEWAY)]);

    run(&pool, &handler).await?;
    assert_eq!(row(&pool).await?, Some((1, false, true)));

    fake.listing_replies([listed()]);
    run(&pool, &handler).await?;

    assert_eq!(fake.methods(), ["POST", "GET"]);
    assert_eq!(row(&pool).await?, None);
    assert!(owned(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unclear_create_missing_on_soundcloud_is_sent_again(pool: PgPool) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    fake.create_replies([Reply::Status(StatusCode::BAD_GATEWAY), created()]);

    run(&pool, &handler).await?;
    run(&pool, &handler).await?;

    assert_eq!(fake.methods(), ["POST", "GET", "POST"]);
    assert_eq!(row(&pool).await?, None);
    assert!(owned(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_create_answer_too_large_to_read_is_found_in_the_listing_without_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    let huge = json!({"urn": CREATED, "description": "x".repeat(1024 * 1024)}).to_string();
    fake.create_replies([Reply::Body(StatusCode::CREATED, huge)]);

    run(&pool, &handler).await?;
    assert_eq!(row(&pool).await?, Some((1, false, true)));

    fake.listing_replies([listed()]);
    run(&pool, &handler).await?;

    let calls = fake.calls();
    assert_eq!(fake.methods(), ["POST", "GET"]);
    assert!(
        calls[1]
            .starts_with("GET /me/playlists?show_tracks=false&linked_partitioning=true&limit=50 ")
    );
    assert_eq!(row(&pool).await?, None);
    let tracks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM playlist_track_projection WHERE playlist_urn = $1",
    )
    .bind(CREATED)
    .fetch_one(&pool)
    .await?;
    assert_eq!(tracks, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unclear_create_keeps_its_mark_through_a_lost_login_and_both_pauses(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    fake.create_replies([Reply::Status(StatusCode::BAD_GATEWAY), created()]);
    run(&pool, &handler).await?;

    sqlx::query("UPDATE sync_queue SET next_run_at = now()")
        .execute(&pool)
        .await?;
    let mutation = handler
        .repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("the create was not claimed"))?;
    sqlx::query(
        "UPDATE soundcloud_connections SET last_refresh_error_kind = 'reauthorization_required'",
    )
    .execute(&pool)
    .await?;
    handler.execute(mutation).await?;
    assert_eq!(row(&pool).await?, Some((1, false, true)));
    assert_eq!(fake.methods(), ["POST"]);
    sqlx::query("UPDATE soundcloud_connections SET last_refresh_error_kind = NULL")
        .execute(&pool)
        .await?;

    let app: uuid::Uuid = sqlx::query_scalar("SELECT oauth_app_id FROM soundcloud_connections")
        .fetch_one(&pool)
        .await?;
    let app_action = (app, "playlist_create".to_owned());
    handler.write_pauses.pause(app_action.clone(), 600);
    run(&pool, &handler).await?;
    assert_eq!(row(&pool).await?, Some((1, false, true)));
    assert_eq!(fake.methods(), ["POST", "GET"]);
    handler.write_pauses.succeeded(&app_action, None);

    let account_action = ("17".to_owned(), "playlist_create".to_owned());
    handler.account_pauses.pause(account_action.clone(), 600);
    run(&pool, &handler).await?;
    assert_eq!(row(&pool).await?, Some((1, false, true)));
    assert_eq!(fake.methods(), ["POST", "GET", "GET"]);
    handler.account_pauses.succeeded(&account_action, None);

    run(&pool, &handler).await?;
    assert_eq!(fake.methods(), ["POST", "GET", "GET", "GET", "POST"]);
    assert_eq!(row(&pool).await?, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_listing_that_keeps_failing_parks_the_create_after_five_tries(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    fake.create_replies([Reply::Status(StatusCode::BAD_GATEWAY)]);
    fake.listing_replies((0..8).map(|_| Reply::Status(StatusCode::FORBIDDEN)));

    run(&pool, &handler).await?;
    for _ in 0..4 {
        run(&pool, &handler).await?;
    }

    assert_eq!(row(&pool).await?, Some((5, true, true)));
    assert_eq!(fake.methods(), ["POST", "GET", "GET", "GET", "GET"]);
    assert!(handler.repository.claim(1).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_listing_refused_for_the_token_is_read_again_with_the_renewed_one(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    fake.create_replies([Reply::Status(StatusCode::BAD_GATEWAY), created()]);
    fake.listing_replies([Reply::RenewToken]);

    run(&pool, &handler).await?;
    run(&pool, &handler).await?;

    let tokens: Vec<String> = fake
        .calls()
        .iter()
        .map(|call| {
            let mut parts = call.split(' ');
            format!(
                "{} {}",
                parts.next().unwrap_or_default(),
                parts.next_back().unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        tokens,
        ["POST access", "GET access", "GET fresh", "POST fresh"]
    );
    assert_eq!(row(&pool).await?, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn five_unclear_posts_park_the_create(pool: PgPool) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    fake.create_replies((0..5).map(|_| Reply::Status(StatusCode::GATEWAY_TIMEOUT)));

    for _ in 0..5 {
        run(&pool, &handler).await?;
    }

    assert_eq!(row(&pool).await?, Some((5, true, true)));
    assert_eq!(
        fake.methods(),
        [
            "POST", "GET", "POST", "GET", "POST", "GET", "POST", "GET", "POST"
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unclear_create_without_a_title_is_parked_at_once(pool: PgPool) -> anyhow::Result<()> {
    let (fake, handler) = setup(&pool).await?;
    sqlx::query("UPDATE sync_queue SET payload = '{\"playlist\": {\"sharing\": \"private\"}}'")
        .execute(&pool)
        .await?;
    fake.create_replies([Reply::Status(StatusCode::BAD_GATEWAY)]);

    run(&pool, &handler).await?;
    run(&pool, &handler).await?;

    let last_error: Option<String> = sqlx::query_scalar("SELECT last_error FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(row(&pool).await?, Some((5, true, true)));
    assert_eq!(fake.methods(), ["POST"]);
    assert_eq!(
        last_error.as_deref(),
        Some(
            "invalid sync action payload: playlist title is missing, the created playlist cannot be looked up"
        )
    );
    Ok(())
}
