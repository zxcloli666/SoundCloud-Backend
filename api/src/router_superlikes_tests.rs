use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{app, get, seed_tracks, session};

const NO_REDIS: &str = "redis://127.0.0.1:1";

async fn send(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::builder()
        .method(method)
        .uri(uri)
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

fn urns(page: &Value) -> Vec<String> {
    page["collection"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|track| track["urn"].as_str().map(str::to_owned))
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn a_superlike_likes_the_track_and_dies_with_the_like(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5", "6"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;

    let (status, body) = send(&app, session, "POST", "/superlikes/soundcloud:tracks:5").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"superliked": true, "used": 1, "limit": null}));
    let (status, _) = send(&app, session, "POST", "/superlikes/6").await?;
    assert_eq!(status, StatusCode::OK);

    let queued: Vec<String> = sqlx::query_scalar(
        "SELECT target_urn FROM sync_queue WHERE action_type = 'like_track' ORDER BY target_urn",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(queued, vec!["soundcloud:tracks:5", "soundcloud:tracks:6"]);

    let (status, page) = get(&app, session, "/superlikes").await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        urns(&page),
        vec!["soundcloud:tracks:6", "soundcloud:tracks:5"]
    );
    assert_eq!(page["collection"][0]["user_superlike"], true);
    assert_eq!(page["collection"][0]["user_favorite"], true);

    let (_, track) = get(&app, session, "/tracks/soundcloud:tracks:5").await?;
    assert_eq!(track["user_superlike"], true, "{track}");

    let (status, body) = send(&app, session, "DELETE", "/superlikes/6").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"superliked": false, "used": 1, "limit": null}));
    let liked: bool =
        sqlx::query_scalar("SELECT wanted_state FROM user_likes_tracks WHERE sc_track_id = '6'")
            .fetch_one(&pool)
            .await?;
    assert!(liked, "removing the superlike keeps the like");

    let (status, _) = send(&app, session, "DELETE", "/likes/tracks/5").await?;
    assert_eq!(status, StatusCode::OK);
    let (_, quota) = get(&app, session, "/superlikes/quota").await?;
    assert_eq!(quota, json!({"used": 0, "limit": null}));
    let (_, page) = get(&app, session, "/superlikes").await?;
    assert!(urns(&page).is_empty());

    let (status, _) = send(&app, session, "POST", "/superlikes/soundcloud:users:5").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_superlikes_list_pages_newest_first(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["1", "2", "3"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    for id in ["1", "2", "3"] {
        let (status, _) = send(&app, session, "POST", &format!("/superlikes/{id}")).await?;
        assert_eq!(status, StatusCode::OK);
    }

    let (_, first) = get(&app, session, "/superlikes?limit=2").await?;
    assert_eq!(
        urns(&first),
        vec!["soundcloud:tracks:3", "soundcloud:tracks:2"]
    );
    let next = first["next_href"].as_str().unwrap_or_default().to_owned();
    assert!(next.starts_with("?limit=2&cursor="), "{next}");

    let (status, second) = get(&app, session, &format!("/superlikes{next}")).await?;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(urns(&second), vec!["soundcloud:tracks:1"]);
    assert_eq!(second["next_href"], Value::Null);
    Ok(())
}
