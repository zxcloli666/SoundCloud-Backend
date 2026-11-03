use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{app, get, seed_tracks, session, soundcloud_hits, state};
use crate::modules::subscriptions::SubscriptionsService;

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

async fn free_app(pool: &PgPool) -> anyhow::Result<axum::Router> {
    let mut state = state(pool, soundcloud_hits(), NO_REDIS).await?;
    state.subscriptions = SubscriptionsService::new(pool.clone(), false);
    Ok(crate::router::build(state))
}

async fn count(pool: &PgPool, sql: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(sql).fetch_one(pool).await?)
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
    assert_eq!(next, "?limit=2&offset=2");

    let (status, second) = get(&app, session, &format!("/superlikes{next}")).await?;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(urns(&second), vec!["soundcloud:tracks:1"]);
    assert_eq!(second["next_href"], Value::Null);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_free_listener_at_ten_gets_402_and_the_track_stays_unliked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let ids: Vec<String> = (1..=11).map(|id| id.to_string()).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    seed_tracks(&pool, &refs).await?;
    let app = free_app(&pool).await?;
    let session = session(&pool).await?;
    for id in &ids[..10] {
        let (status, body) = send(&app, session, "POST", &format!("/superlikes/{id}")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let liked_before = count(&pool, "SELECT count(*) FROM user_likes_tracks").await?;
    let queued_before = count(&pool, "SELECT count(*) FROM sync_queue").await?;

    let (status, body) = send(&app, session, "POST", "/superlikes/11").await?;

    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(body["code"], "superlike_limit_reached", "{body}");
    let liked_after = count(&pool, "SELECT count(*) FROM user_likes_tracks").await?;
    let queued_after = count(&pool, "SELECT count(*) FROM sync_queue").await?;
    assert_eq!((liked_after, queued_after), (liked_before, queued_before));
    let (_, quota) = get(&app, session, "/superlikes/quota").await?;
    assert_eq!(quota, json!({"used": 10, "limit": 10}));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn liking_a_liked_track_again_keeps_its_superlike_and_its_place(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let (status, _) = send(&app, session, "POST", "/superlikes/5").await?;
    assert_eq!(status, StatusCode::OK);
    let liked_at = "SELECT extract(epoch FROM created_at)::float8 FROM user_likes_tracks";
    let before: f64 = sqlx::query_scalar(liked_at).fetch_one(&pool).await?;
    let sync_generation = "SELECT generation FROM sync_queue WHERE action_type = 'like_track'";
    let queued: i64 = sqlx::query_scalar(sync_generation).fetch_one(&pool).await?;

    let (status, body) = send(&app, session, "POST", "/likes/tracks/soundcloud:tracks:5").await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    let after: f64 = sqlx::query_scalar(liked_at).fetch_one(&pool).await?;
    assert_eq!(after, before);
    let (_, quota) = get(&app, session, "/superlikes/quota").await?;
    assert_eq!(quota, json!({"used": 1, "limit": null}));
    let events = count(
        &pool,
        "SELECT count(*) FROM user_events WHERE event_type = 'like'",
    )
    .await?;
    assert_eq!(events, 1);
    let requeued: i64 = sqlx::query_scalar(sync_generation).fetch_one(&pool).await?;
    assert!(
        requeued > queued,
        "a repeat like is a new intent, the node cannot know its copy is current"
    );
    Ok(())
}
