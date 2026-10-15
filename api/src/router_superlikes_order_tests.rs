use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::router_identity_tests::send;
use super::router_likes_order_tests::{NO_REDIS, expected, move_after, move_to_top};
use super::router_search_tests::{app, get, seed_tracks, session, urns};

const ORDER: &str = "/superlikes/order";

async fn superlike(app: &axum::Router, session: Uuid, ids: &[&str]) -> anyhow::Result<()> {
    for id in ids {
        let (status, _, body) =
            send(app, session, "POST", &format!("/superlikes/{id}"), None).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    Ok(())
}

async fn order(app: &axum::Router, session: Uuid) -> anyhow::Result<Vec<String>> {
    let (status, body) = get(app, session, "/superlikes?limit=200").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(urns(&body)
        .into_iter()
        .map(|urn| urn.trim_start_matches("soundcloud:tracks:").to_owned())
        .collect())
}

async fn reorder(
    app: &axum::Router,
    session: Uuid,
    body: serde_json::Value,
) -> anyhow::Result<(StatusCode, serde_json::Value)> {
    let (status, _, body) = send(app, session, "POST", ORDER, Some(body)).await?;
    Ok((status, body))
}

async fn ranked(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT sc_track_id FROM user_superlike_ranks ORDER BY sc_track_id")
            .fetch_all(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn the_superlikes_order_survives_moves_new_superlikes_removals_and_reset(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_tracks(&pool, &["1", "2", "3", "4", "5"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    superlike(&app, session, &["1", "2", "3"]).await?;
    assert_eq!(order(&app, session).await?, expected(&["3", "2", "1"]));

    let (status, _) = reorder(&app, session, move_after("3", "1")).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(order(&app, session).await?, expected(&["2", "1", "3"]));
    let (status, _) = reorder(
        &app,
        session,
        json!({"action": "move_to_bottom", "track": "2"}),
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(order(&app, session).await?, expected(&["1", "3", "2"]));

    superlike(&app, session, &["4"]).await?;
    assert_eq!(order(&app, session).await?, expected(&["4", "1", "3", "2"]));

    let (status, _, _) = send(&app, session, "DELETE", "/superlikes/3", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ranked(&pool).await?, expected(&["1", "2"]));
    superlike(&app, session, &["3"]).await?;
    assert_eq!(order(&app, session).await?, expected(&["3", "4", "1", "2"]));

    let (status, _) = reorder(&app, session, move_to_top("2")).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(order(&app, session).await?, expected(&["2", "3", "4", "1"]));

    let (status, _, _) = send(&app, session, "DELETE", "/likes/tracks/4", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(order(&app, session).await?, expected(&["2", "3", "1"]));

    let (status, _) = reorder(&app, session, json!({"action": "reset"})).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(ranked(&pool).await?.is_empty());
    assert_eq!(order(&app, session).await?, expected(&["3", "2", "1"]));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_move_of_a_track_that_is_no_superlike_is_a_coded_404(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["1", "2"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    superlike(&app, session, &["1"]).await?;

    let (status, body) = reorder(&app, session, move_to_top("2")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "superlike_not_found", "{body}");
    let (status, body) = reorder(&app, session, move_after("1", "2")).await?;
    assert_eq!(
        (status, body["code"].clone()),
        (StatusCode::NOT_FOUND, json!("superlike_not_found"))
    );
    let (status, _) = reorder(&app, session, move_after("1", "1")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = reorder(&app, session, json!({"action": "shuffle"})).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(ranked(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn pages_follow_the_order_through_the_offset_link(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["1", "2", "3"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    superlike(&app, session, &["1", "2", "3"]).await?;
    let (status, _) = reorder(&app, session, move_to_top("1")).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, first) = get(&app, session, "/superlikes?limit=2").await?;
    assert_eq!(
        urns(&first),
        vec!["soundcloud:tracks:1", "soundcloud:tracks:3"]
    );
    assert_eq!(first["next_href"], "?limit=2&offset=2");
    let (_, second) = get(&app, session, "/superlikes?limit=2&offset=2").await?;
    assert_eq!(urns(&second), vec!["soundcloud:tracks:2"]);
    assert_eq!(second["next_href"], serde_json::Value::Null);
    Ok(())
}
