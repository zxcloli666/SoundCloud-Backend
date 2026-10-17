use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{LISTENER, app};

async fn session_without_a_refreshable_login(pool: &PgPool) -> anyhow::Result<Uuid> {
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'spare', 'spare', 'secret', 'https://app.test/')",
    )
    .bind(Uuid::now_v7())
    .execute(pool)
    .await?;
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, 'Listener', 'access', 'refresh', now() - interval '1 hour', '')",
    )
    .bind(connection)
    .bind(LISTENER)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

#[sqlx::test(migrations = "./migrations")]
async fn an_upload_is_refused_before_the_file_arrives_when_soundcloud_needs_a_new_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    let session = session_without_a_refreshable_login(&pool).await?;
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let file_never_arrives =
        Body::from_stream(futures::stream::pending::<Result<Bytes, std::io::Error>>());
    let request = Request::post("/tracks/upload")
        .header("x-session-id", session.to_string())
        .header("content-type", "multipart/form-data; boundary=upload")
        .body(file_never_arrives)?;

    let response = tokio::time::timeout(Duration::from_secs(10), app.oneshot(request)).await??;
    let status = response.status();
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "soundcloud_reauthorization_required");
    Ok(())
}
