use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{LISTENER, app, session};

async fn retry(app: &axum::Router, session: Uuid) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::builder()
        .method("POST")
        .uri("/me/sync/retry")
        .header("x-session-id", session.to_string())
        .body(Body::empty())?;
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn dead(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT action_type FROM sync_queue WHERE dead ORDER BY action_type")
            .fetch_all(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn a_retry_keeps_every_dead_edit_when_the_playlist_re_read_is_not_queued(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let session = session(&pool).await?;
    sqlx::query("ALTER TABLE background_jobs ADD CHECK (kind <> 'playlists.observe_shadow')")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, payload, dead, failed_at, retry_count)
         VALUES ($1, 'playlist_membership', 'soundcloud:playlists:77',
                 '{\"tracks\": [\"1\"], \"fingerprint\": \"aa\", \"reconcile_generation\": 3}',
                 true, now() - interval '5 minutes', 5),
                ($1, 'playlist_update', 'soundcloud:playlists:78', '{}',
                 true, now() - interval '5 minutes', 5)",
    )
    .bind(LISTENER)
    .execute(&pool)
    .await?;

    let (status, _) = retry(&app, session).await?;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        dead(&pool).await?,
        ["playlist_membership", "playlist_update"]
    );

    sqlx::query("DELETE FROM sync_queue WHERE action_type = 'playlist_membership'")
        .execute(&pool)
        .await?;
    let (status, body) = retry(&app, session).await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"revived": 1}));
    assert!(dead(&pool).await?.is_empty());
    Ok(())
}
