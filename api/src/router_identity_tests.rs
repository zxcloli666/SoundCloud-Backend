use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{LISTENER, app, get, seed_tracks, session};

const NO_REDIS: &str = "redis://127.0.0.1:1";

async fn send(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Option<String>, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-session-id", session.to_string());
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = app.clone().oneshot(request.body(body)?).await?;
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        location,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

#[sqlx::test(migrations = "./migrations")]
async fn a_bare_stream_id_is_forwarded_as_its_canonical_urn(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["42"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    for uri in ["/tracks/42/stream", "/tracks/soundcloud:tracks:42/stream"] {
        let (status, location, _) = send(&app, session, "GET", uri, None).await?;
        assert_eq!(status, StatusCode::TEMPORARY_REDIRECT, "{uri}");
        let location = location.expect("a stream location");
        assert!(
            location.starts_with("http://127.0.0.1:1/stream/soundcloud:tracks:42?ticket="),
            "{uri}: {location}"
        );
    }
    for uri in [
        "/tracks/soundcloud:users:42/stream",
        "/tracks/042/stream",
        "/tracks/x/stream",
    ] {
        let (status, location, _) = send(&app, session, "GET", uri, None).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(location, None, "{uri}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn history_is_written_canonical_and_read_back_with_a_urn(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO listening_history (soundcloud_user_id, sc_track_id, title, artist_name, duration, played_at)
         VALUES ($1, '43', 'Legacy', 'Someone', 1000, now() - interval '1 day')",
    )
    .bind(LISTENER)
    .execute(&pool)
    .await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let play = |id: &str| json!({"scTrackId": id, "title": "Song", "artistName": "Artist", "duration": 1000});
    for id in ["42", "soundcloud:tracks:42"] {
        let (status, _, body) = send(&app, session, "POST", "/history", Some(play(id))).await?;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
    }
    let (status, _, body) = send(
        &app,
        session,
        "POST",
        "/history",
        Some(play("soundcloud:users:42")),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT sc_track_id FROM listening_history WHERE played_at > now() - interval '1 hour'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(stored, vec!["soundcloud:tracks:42".to_owned()]);

    let (status, body) = get(&app, session, "/history").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows: Vec<(String, String)> = body["collection"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            (
                row["scTrackId"].as_str().unwrap_or_default().to_owned(),
                row["trackUrn"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "soundcloud:tracks:42".to_owned(),
                "soundcloud:tracks:42".to_owned()
            ),
            ("43".to_owned(), "soundcloud:tracks:43".to_owned()),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn dislike_ids_also_come_as_urns(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    for id in ["soundcloud:tracks:42", "7"] {
        let (status, _, body) =
            send(&app, session, "POST", &format!("/dislikes/{id}"), None).await?;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
    }
    let (status, body) = get(&app, session, "/dislikes/ids").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut ids: Vec<String> = serde_json::from_value(body["ids"].clone())?;
    let mut urns: Vec<String> = serde_json::from_value(body["urns"].clone())?;
    ids.sort();
    urns.sort();
    assert_eq!(ids, vec!["42".to_owned(), "7".to_owned()]);
    assert_eq!(
        urns,
        vec![
            "soundcloud:tracks:42".to_owned(),
            "soundcloud:tracks:7".to_owned()
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn likes_are_keyed_by_one_canonical_form(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let (status, _, body) = send(&app, session, "POST", "/likes/playlists/7", None).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = send(&app, session, "POST", "/likes/tracks/42", None).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, _) = send(
        &app,
        session,
        "POST",
        "/likes/tracks/soundcloud:users:42",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let playlist: String = sqlx::query_scalar("SELECT playlist_urn FROM user_likes_playlists")
        .fetch_one(&pool)
        .await?;
    assert_eq!(playlist, "soundcloud:playlists:7");
    let track: String = sqlx::query_scalar("SELECT sc_track_id FROM user_likes_tracks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(track, "42");
    let targets: Vec<String> =
        sqlx::query_scalar("SELECT target_urn FROM sync_queue ORDER BY target_urn")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        targets,
        vec![
            "soundcloud:playlists:7".to_owned(),
            "soundcloud:tracks:42".to_owned()
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn dislikes_are_read_back_from_the_catalog_not_from_the_posted_body(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let posts = [
        (
            "5",
            Some(json!({"urn": "soundcloud:tracks:5", "title": "Forged"})),
        ),
        (
            "6",
            Some(json!({"urn": "soundcloud:tracks:6", "title": "Six"})),
        ),
        (
            "7",
            Some(json!({"urn": "soundcloud:tracks:8", "title": "Mismatch"})),
        ),
    ];
    for (id, body) in posts {
        let (status, _, body) =
            send(&app, session, "POST", &format!("/dislikes/{id}"), body).await?;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
    }
    let (status, body) = get(&app, session, "/dislikes").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["collection"].as_array().cloned().unwrap_or_default();
    let found: Vec<(String, String)> = items
        .iter()
        .map(|item| {
            (
                item["urn"].as_str().unwrap_or_default().to_owned(),
                item["title"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert!(
        found.contains(&(
            "soundcloud:tracks:5".to_owned(),
            "Midnight Train 5".to_owned()
        )),
        "{found:?}"
    );
    assert!(
        found.contains(&("soundcloud:tracks:6".to_owned(), "Six".to_owned())),
        "{found:?}"
    );
    assert_eq!(found.len(), 2, "{found:?}");
    let catalog = items
        .iter()
        .find(|item| item["urn"] == "soundcloud:tracks:5")
        .unwrap();
    assert!(catalog["_scd_meta"].is_object());

    let titles: Vec<String> = sqlx::query_scalar("SELECT title FROM tracks ORDER BY sc_track_id")
        .fetch_all(&pool)
        .await?;
    assert_eq!(titles, vec!["Midnight Train 5".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_like_body_never_becomes_catalog_data(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5"]).await?;
    sqlx::query("UPDATE tracks SET index_priority = 5, storage_priority = 5")
        .execute(&pool)
        .await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    for (id, title) in [("5", "Forged"), ("42", "Invented")] {
        let body =
            json!({"urn": format!("soundcloud:tracks:{id}"), "title": title, "duration": 1000});
        let (status, _, body) = send(
            &app,
            session,
            "POST",
            &format!("/likes/tracks/{id}"),
            Some(body),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let rows: Vec<(String, String, i16)> = sqlx::query_as(
        "SELECT sc_track_id, title, index_priority FROM tracks ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        vec![("5".to_owned(), "Midnight Train 5".to_owned(), 1)],
        "an existing track is only promoted, a missing one waits for SoundCloud"
    );
    Ok(())
}
