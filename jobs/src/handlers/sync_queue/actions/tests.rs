use super::*;
use crate::config::SyncQueueConfig;
use axum::{
    Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    routing::{delete, post, put},
};
use std::time::Duration;

fn config(address: std::net::SocketAddr) -> anyhow::Result<SyncQueueConfig> {
    Ok(SyncQueueConfig {
        api_url: format!("http://{address}/").parse()?,
        proxy_url: None,
        storage_url: "http://127.0.0.1:1/".parse()?,
        storage_token: String::new().into(),
        concurrency: 1,
        claim_batch: 1,
        lease_duration: Duration::from_secs(60),
        like_interval: Duration::from_secs(5),
    })
}

fn mutation(action_type: &str, target_urn: &str, payload: Option<Value>) -> ClaimedMutation {
    ClaimedMutation {
        id: uuid::Uuid::nil(),
        user_id: "17".to_owned(),
        action_type: action_type.to_owned(),
        target_urn: target_urn.to_owned(),
        payload,
        retry_count: 0,
        generation: 1,
        lease_id: uuid::Uuid::nil(),
        lease_generation: 1,
        remote_attempted_generation: Some(1),
        remote_completed_generation: None,
        remote_result: None,
        created_at: chrono::Utc::now(),
        star: false,
    }
}

fn missing(Path(id): Path<String>) -> StatusCode {
    if id.ends_with(":410") {
        StatusCode::GONE
    } else {
        StatusCode::NOT_FOUND
    }
}

#[tokio::test]
async fn a_like_or_follow_of_a_missing_target_is_done_but_a_missing_playlist_edit_is_not()
-> anyhow::Result<()> {
    let app = Router::new()
        .route("/likes/tracks/{id}", post(|id| async { missing(id) }))
        .route("/likes/playlists/{id}", post(|id| async { missing(id) }))
        .route("/me/followings/{id}", put(|id| async { missing(id) }))
        .route("/playlists/{id}", put(|id| async { missing(id) }));
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
    let client = SoundCloudClient::new(&config(address)?)?;
    let result: anyhow::Result<()> = async {
        for (action, target) in [
            ("like_track", "soundcloud:tracks:404"),
            ("like_track", "soundcloud:tracks:410"),
            ("like_playlist", "soundcloud:playlists:404"),
            ("follow_user", "soundcloud:users:410"),
        ] {
            let done = execute_remote(&client, &mutation(action, target, None), "token").await?;
            assert!(done.is_null(), "{action} {target}");
        }
        let edit = mutation(
            "playlist_update",
            "soundcloud:playlists:404",
            Some(serde_json::json!({"playlist": {"title": "Renamed"}})),
        );
        let failed = execute_remote(&client, &edit, "token").await;
        assert!(matches!(
            failed,
            Err(ActionError::SoundCloud(SoundCloudError::Api { status, .. })) if status.as_u16() == 404
        ));
        Ok(())
    }
    .await;
    let _ = stop.send(());
    server.await??;
    result
}

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
    let client = SoundCloudClient::new(&config(address)?)?;
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
