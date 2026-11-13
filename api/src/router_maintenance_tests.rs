use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::stream;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend_contracts::{JOB_INGRESS_STREAM, JOB_INGRESS_SUBJECT};
use futures::StreamExt;
use serde_json::Value;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use super::router_search_tests::{config, nats_url, redis_url, soundcloud_hits, state_with};
use crate::bus::nats::NatsService;

const ADMIN: &str = "maintenance-admin-token";

async fn app(pool: &PgPool) -> anyhow::Result<axum::Router> {
    let mut config = config(&redis_url());
    config.nats.url = nats_url();
    config.admin.token = ADMIN.to_owned().into();
    let state = state_with(pool, soundcloud_hits(), config).await?;
    Ok(crate::router::build(state))
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("x-admin-token", token);
    }
    let response = app.clone().oneshot(request.body(Body::empty())?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn ingress() -> anyhow::Result<(Arc<NatsService>, async_nats::Subscriber)> {
    let nats = NatsService::connect(&nats_url(), CancellationToken::new()).await?;
    nats.wait_connected(Duration::from_secs(2)).await;
    nats.jetstream()
        .get_or_create_stream(stream::Config {
            name: JOB_INGRESS_STREAM.to_owned(),
            subjects: vec![JOB_INGRESS_SUBJECT.to_owned()],
            storage: stream::StorageType::Memory,
            ..Default::default()
        })
        .await?;
    let subscription = nats.subscribe(JOB_INGRESS_SUBJECT).await?;
    nats.jetstream().client().flush().await?;
    Ok((nats, subscription))
}

async fn command_for(
    subscription: &mut async_nats::Subscriber,
    dedup_key: &str,
) -> anyhow::Result<Value> {
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(message) = subscription.next().await {
            let body: Value = serde_json::from_slice(&message.payload).unwrap_or(Value::Null);
            if body["payload"]["dedupKey"] == dedup_key {
                return Some(body["payload"].clone());
            }
        }
        None
    })
    .await?;
    found.ok_or_else(|| anyhow::anyhow!("the ingress subscription closed"))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_lyrics_repair_routes_enqueue_their_own_job_kinds(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool).await?;
    let (_nats, mut subscription) = ingress().await?;

    for (path, kind, job_kind) in [
        (
            "/admin/maintenance/lyrics-refetch",
            "lyrics_refetch",
            "admin.lyrics_refetch",
        ),
        (
            "/admin/maintenance/lyrics-description",
            "lyrics_description",
            "admin.lyrics_description",
        ),
    ] {
        let (status, _) = send(&app, "POST", path, None).await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");

        let (status, body) = send(&app, "POST", path, Some(ADMIN)).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["kind"], kind);
        assert_eq!(body["started"], true);

        let command = command_for(&mut subscription, kind).await?;
        assert_eq!(command["kind"], job_kind);
        assert_eq!(command["enqueueIfAbsent"], true);

        sqlx::query(
            "INSERT INTO admin_maintenance_runs (kind, run_id, status, phase)
             VALUES ($1, $2, 'running', 'drain')",
        )
        .bind(kind)
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await?;
        let (status, body) = send(&app, "POST", path, Some(ADMIN)).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["started"], false);
    }

    let (status, body) = send(&app, "GET", "/admin/maintenance/status", Some(ADMIN)).await?;
    assert_eq!(status, StatusCode::OK);
    let runs: Vec<(&str, &str)> = body
        .as_array()
        .map(|runs| {
            runs.iter()
                .filter_map(|run| Some((run["kind"].as_str()?, run["phase"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        runs,
        [("lyrics_description", "drain"), ("lyrics_refetch", "drain")]
    );
    Ok(())
}
