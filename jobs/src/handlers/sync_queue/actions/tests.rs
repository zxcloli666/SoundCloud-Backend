use super::*;
use crate::config::SyncQueueConfig;
use axum::{
    Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    routing::delete,
};
use std::time::Duration;

#[tokio::test]
async fn remote_deletion_accepts_absence_but_preserves_authorization_and_server_failures()
-> anyhow::Result<()> {
    let app = Router::new().route(
        "/tracks/{status}",
        delete(|Path(status): Path<u16>, headers: HeaderMap| async move {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("OAuth token")
            );
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let config = SyncQueueConfig {
        api_url: format!("http://{address}/").parse()?,
        proxy_url: None,
        storage_url: "http://127.0.0.1:1/".parse()?,
        storage_token: String::new().into(),
        concurrency: 1,
        claim_batch: 1,
        lease_duration: Duration::from_secs(60),
    };
    let client = SoundCloudClient::new(&config)?;
    let result: anyhow::Result<()> = async {
        for status in [204, 404, 410] {
            assert!(delete_remote(&client, &format!("/tracks/{status}"), "token").await?.is_null());
        }
        for status in [401, 429, 503] {
            let error = delete_remote(&client, &format!("/tracks/{status}"), "token").await;
            assert!(matches!(error, Err(ActionError::SoundCloud(SoundCloudError::Api { status: actual, .. })) if actual.as_u16() == status));
        }
        Ok(())
    }.await;
    let _ = stop.send(());
    server.await??;
    result
}
