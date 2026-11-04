use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::router_identity_tests::send;
use super::router_search_tests::{app, redis_url, session};
use crate::modules::search::catalog_tests::{seed, song};

fn items(pairs: &[(&str, &str)]) -> Value {
    json!({"items": pairs.iter().map(|(artist, title)| json!({"artist": artist, "title": title})).collect::<Vec<_>>()})
}

async fn post(
    app: &axum::Router,
    session: Uuid,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let (status, _, body) = send(app, session, "POST", "/tracks/match", Some(body)).await?;
    Ok((status, body))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn results_keep_the_order_and_length_of_the_items(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            song(1, "Umbrella", "Rihanna", 7_000_000),
            song(2, "Enter Sandman", "Metallica", 8_000_000),
        ],
    )
    .await?;
    let app = app(&pool, &redis_url()).await?;
    let session = session(&pool).await?;
    let (status, body) = post(
        &app,
        session,
        items(&[
            ("Metallica", "Enter Sandman"),
            ("Nobody", "Unknown song"),
            ("Rihanna", "Umbrella (Official Video)"),
            ("metallica", "ENTER SANDMAN"),
        ]),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"results": [
            {"urn": "soundcloud:tracks:2"},
            {"urn": null},
            {"urn": "soundcloud:tracks:1"},
            {"urn": "soundcloud:tracks:2"},
        ]})
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_hundred_items_pass_and_a_hundred_and_one_do_not(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session(&pool).await?;
    let many = |n: usize| json!({"items": (0..n).map(|i| json!({"artist": "a", "title": format!("t{i}"), "duration_ms": null})).collect::<Vec<_>>()});
    let (status, body) = post(&app, session, many(100)).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["results"].as_array().map(Vec::len), Some(100));
    let (status, _) = post(&app, session, many(101)).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn malformed_or_empty_requests_are_rejected(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session(&pool).await?;
    for body in [
        json!({"items": []}),
        json!({}),
        json!({"items": "x"}),
        json!([1]),
    ] {
        let (status, _) = post(&app, session, body.clone()).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_missing_session_is_unauthorized_like_the_search(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let (status, _) = post(&app, Uuid::now_v7(), items(&[("a", "b")])).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}
