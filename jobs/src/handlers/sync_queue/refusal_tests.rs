use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use sqlx::PgPool;
use uuid::Uuid;

use super::SyncQueueHandler;
use super::unclear_create_tests::{handler, run};

async fn serve() -> anyhow::Result<std::net::SocketAddr> {
    let app = Router::new()
        .route(
            "/likes/tracks/{id}",
            post(|| async { StatusCode::NOT_FOUND }),
        )
        .route(
            "/likes/playlists/{id}",
            post(|| async { StatusCode::CREATED }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(address)
}

async fn setup(
    pool: &PgPool,
    action_type: &str,
    target_urn: &str,
) -> anyhow::Result<(Uuid, Uuid, SyncQueueHandler)> {
    crate::db::migrations::run_core(pool, None).await?;
    super::track_tests::connect_owner(pool).await?;
    let app: Uuid = sqlx::query_scalar("SELECT oauth_app_id FROM soundcloud_connections")
        .fetch_one(pool)
        .await?;
    let other = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'other', $2, 'secret', 'https://app.test/')",
    )
    .bind(other)
    .bind(other.to_string())
    .execute(pool)
    .await?;
    let handler = handler(pool, serve().await?)?;
    let pauses = [
        (app, "playlist_create", "subscription_required"),
        (app, "like_track", "subscription_required"),
        (app, "comment", "subscription_required"),
        (app, "follow_user", "rate_limited"),
        (other, "playlist_create", "subscription_required"),
    ];
    for (app, action, reason) in pauses {
        sqlx::query(
            "INSERT INTO sync_queue_action_pauses (oauth_app_id, action_type, paused_until, strikes, reason)
             VALUES ($1, $2, now(), 1, $3)",
        )
        .bind(app)
        .bind(action)
        .bind(reason)
        .execute(pool)
        .await?;
        handler
            .write_pauses
            .restore((app, action.to_owned()), Duration::from_secs(600), 1);
    }
    sqlx::query("INSERT INTO sync_queue (user_id, action_type, target_urn) VALUES ('17', $1, $2)")
        .bind(action_type)
        .bind(target_urn)
        .execute(pool)
        .await?;
    Ok((app, other, handler))
}

async fn marks(pool: &PgPool, app: Uuid) -> anyhow::Result<Vec<(String, String, bool)>> {
    Ok(sqlx::query_as(
        "SELECT action_type, reason, oauth_app_id = $1
         FROM sync_queue_action_pauses
         ORDER BY 1, 3 DESC",
    )
    .bind(app)
    .fetch_all(pool)
    .await?)
}

async fn queued(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(pool)
        .await?)
}

#[sqlx::test(migrations = false)]
async fn a_written_like_lifts_every_subscription_mark_of_its_app_and_keeps_rate_limits(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (app, other, handler) = setup(&pool, "like_playlist", "soundcloud:playlists:1").await?;
    let paused = |app: Uuid, action: &str| {
        handler
            .write_pauses
            .remaining_seconds(&(app, action.to_owned()))
            .is_some()
    };

    run(&pool, &handler).await?;

    assert_eq!(queued(&pool).await?, 0);
    assert!(!paused(app, "playlist_create"));
    assert!(!paused(app, "like_track"));
    assert!(!paused(app, "comment"));
    assert!(paused(app, "follow_user"));
    assert!(paused(other, "playlist_create"));
    assert_eq!(
        marks(&pool, app).await?,
        [
            ("follow_user".to_owned(), "rate_limited".to_owned(), true),
            (
                "playlist_create".to_owned(),
                "subscription_required".to_owned(),
                false
            ),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_closed_on_a_missing_track_leaves_every_mark_in_place(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (app, _, handler) = setup(&pool, "like_track", "soundcloud:tracks:404").await?;
    handler
        .write_pauses
        .restore((app, "like_track".to_owned()), Duration::ZERO, 1);
    let before = marks(&pool, app).await?;

    run(&pool, &handler).await?;

    assert_eq!(queued(&pool).await?, 0);
    assert_eq!(before.len(), 5);
    assert_eq!(marks(&pool, app).await?, before);
    assert!(
        handler
            .write_pauses
            .remaining_seconds(&(app, "playlist_create".to_owned()))
            .is_some()
    );
    Ok(())
}
