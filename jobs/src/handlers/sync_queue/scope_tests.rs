use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri, header};
use axum::response::IntoResponse;
use sqlx::PgPool;
use uuid::Uuid;

use super::SyncQueueHandler;
use super::unclear_create_tests::{handler, run};

type Calls = Arc<Mutex<Vec<String>>>;

async fn serve() -> anyhow::Result<(std::net::SocketAddr, Calls)> {
    let calls = Calls::default();
    let app = Router::new().fallback(answer).with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((address, calls))
}

async fn answer(State(calls): State<Calls>, method: Method, uri: Uri) -> impl IntoResponse {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(format!("{method} {uri}"));
    if uri.path().starts_with("/likes/tracks/") && !uri.path().ends_with("/ok") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"Rate Limit Exceeded"}"#,
        );
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        "{}",
    )
}

async fn setup(pool: &PgPool) -> anyhow::Result<(Uuid, SyncQueueHandler, Calls)> {
    crate::db::migrations::run_core(pool, None).await?;
    super::track_tests::connect_owner(pool).await?;
    let app: Uuid = sqlx::query_scalar("SELECT oauth_app_id FROM soundcloud_connections")
        .fetch_one(pool)
        .await?;
    for user in ["21", "22", "23", "24"] {
        sqlx::query(
            "INSERT INTO soundcloud_connections (
                 id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
             ) VALUES ($1, $2, $3, 'access', 'refresh', now() + interval '1 day', '')",
        )
        .bind(Uuid::now_v7())
        .bind(user)
        .bind(app)
        .execute(pool)
        .await?;
    }
    let (address, calls) = serve().await?;
    Ok((app, handler(pool, address)?, calls))
}

async fn attempt(
    pool: &PgPool,
    handler: &SyncQueueHandler,
    calls: &Calls,
    user: &str,
    action_type: &str,
    target: &str,
) -> anyhow::Result<bool> {
    sqlx::query("DELETE FROM sync_queue").execute(pool).await?;
    sqlx::query("INSERT INTO sync_queue (user_id, action_type, target_urn) VALUES ($1, $2, $3)")
        .bind(user)
        .bind(action_type)
        .bind(target)
        .execute(pool)
        .await?;
    let before = calls.lock().unwrap_or_else(PoisonError::into_inner).len();
    run(pool, handler).await?;
    let after = calls.lock().unwrap_or_else(PoisonError::into_inner).len();
    Ok(after > before)
}

fn app_pause(handler: &SyncQueueHandler, app: Uuid, group: &str) -> Option<i64> {
    handler
        .write_pauses
        .remaining_seconds(&(app, group.to_owned()))
}

fn open_spacing(handler: &SyncQueueHandler, app: Uuid) {
    handler
        .spacings
        .succeeded(&(app, "like_track".to_owned()), None);
}

#[sqlx::test(migrations = false)]
async fn one_users_429_pauses_only_that_users_likes(pool: PgPool) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;

    assert!(attempt(&pool, &handler, &calls, "21", "like_track", "9").await?);
    open_spacing(&handler, app);

    assert!(!attempt(&pool, &handler, &calls, "21", "like_track", "9").await?);
    assert!(!attempt(&pool, &handler, &calls, "21", "unlike_track", "9").await?);
    assert!(attempt(&pool, &handler, &calls, "21", "playlist_delete", "9").await?);
    assert!(attempt(&pool, &handler, &calls, "22", "like_track", "9").await?);
    assert_eq!(app_pause(&handler, app, "likes"), None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_account_paused_like_takes_no_spacing_slot(pool: PgPool) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;
    assert!(attempt(&pool, &handler, &calls, "21", "like_track", "9").await?);
    open_spacing(&handler, app);

    assert!(!attempt(&pool, &handler, &calls, "21", "like_track", "9").await?);

    assert!(attempt(&pool, &handler, &calls, "22", "like_track", "9").await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn three_users_with_a_429_and_no_success_in_likes_pause_the_app_likes_only(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;

    for user in ["21", "22"] {
        assert!(attempt(&pool, &handler, &calls, user, "like_track", "9").await?);
        open_spacing(&handler, app);
        assert_eq!(app_pause(&handler, app, "likes"), None);
    }
    assert!(attempt(&pool, &handler, &calls, "23", "like_track", "9").await?);

    assert!(app_pause(&handler, app, "likes").is_some());
    assert!(!attempt(&pool, &handler, &calls, "24", "like_track", "9").await?);
    assert!(!attempt(&pool, &handler, &calls, "24", "unlike_track", "9").await?);
    assert!(attempt(&pool, &handler, &calls, "24", "follow_user", "9").await?);
    assert_eq!(app_pause(&handler, app, "follows"), None);
    let mut saved: Vec<String> =
        sqlx::query_scalar("SELECT action_type FROM sync_queue_action_pauses")
            .fetch_all(&pool)
            .await?;
    saved.sort();
    let mut expected = super::pause_group::members("likes");
    expected.sort();
    assert_eq!(saved, expected);
    assert_eq!(saved.len(), 4);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_that_goes_through_keeps_the_429s_per_account(pool: PgPool) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;

    for user in ["21", "22"] {
        assert!(attempt(&pool, &handler, &calls, user, "like_track", "9").await?);
        open_spacing(&handler, app);
    }
    assert!(attempt(&pool, &handler, &calls, "24", "like_track", "ok").await?);
    open_spacing(&handler, app);
    assert!(attempt(&pool, &handler, &calls, "23", "like_track", "9").await?);

    assert_eq!(app_pause(&handler, app, "likes"), None);
    for user in ["21", "22", "23"] {
        assert!(!attempt(&pool, &handler, &calls, user, "like_track", "9").await?);
    }
    open_spacing(&handler, app);
    assert!(attempt(&pool, &handler, &calls, "24", "like_track", "ok").await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_spaces_only_its_own_action(pool: PgPool) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;

    assert!(attempt(&pool, &handler, &calls, "21", "like_track", "ok").await?);

    assert!(!attempt(&pool, &handler, &calls, "22", "like_track", "ok").await?);
    assert!(attempt(&pool, &handler, &calls, "22", "unlike_track", "ok").await?);
    assert!(attempt(&pool, &handler, &calls, "22", "like_playlist", "ok").await?);
    assert_eq!(app_pause(&handler, app, "likes"), None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_restored_row_with_an_action_name_pauses_its_group(pool: PgPool) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue_action_pauses (oauth_app_id, action_type, paused_until, strikes, reason)
         VALUES ($1, 'like_track', now() + interval '10 minutes', 1, 'rate_limited')",
    )
    .bind(app)
    .execute(&pool)
    .await?;

    handler.restore_pauses().await;

    assert!(app_pause(&handler, app, "likes").is_some());
    assert!(!attempt(&pool, &handler, &calls, "21", "unlike_track", "9").await?);
    assert!(attempt(&pool, &handler, &calls, "21", "follow_user", "9").await?);
    Ok(())
}
