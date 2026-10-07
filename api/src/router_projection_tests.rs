use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::router_search_tests::{LISTENER, get, redis_url, session, state, urns};
use crate::sc::read_tests::SearchRelay;

fn track(id: u64) -> Value {
    json!({
        "kind": "track",
        "id": id,
        "urn": format!("soundcloud:tracks:{id}"),
        "title": format!("remote {id}"),
        "duration": 200000,
        "full_duration": 200000,
        "access": "playable",
        "policy": "ALLOW",
        "likes_count": 7,
        "playback_count": 70,
        "user": {
            "kind": "user",
            "id": 300 + id,
            "urn": format!("soundcloud:users:{}", 300 + id),
            "username": format!("artist {id}"),
        }
    })
}

fn relay() -> Arc<SearchRelay> {
    SearchRelay::answering(
        Box::new(|inputs| {
            let collection: Vec<Value> = match inputs["type"].as_str() {
                Some("tracks") => (901..=920).map(track).collect(),
                Some("users") => (901..=920)
                    .map(|id| {
                        json!({"kind": "user", "id": id, "urn": format!("soundcloud:users:{id}"), "username": format!("remote {id}")})
                    })
                    .collect(),
                _ => (901..=920)
                    .map(|id| {
                        json!({
                            "kind": "playlist",
                            "id": id,
                            "urn": format!("soundcloud:playlists:{id}"),
                            "title": format!("remote {id}"),
                            "track_count": 3,
                            "user": {"urn": "soundcloud:users:77", "username": "owner"},
                        })
                    })
                    .collect(),
            };
            Some(
                json!({"ok": true, "collection": collection, "next_href": "https://api-v2.soundcloud.com/search/tracks?offset=20"}),
            )
        }),
        500,
        json!({}),
    )
}

async fn app(pool: &PgPool) -> anyhow::Result<axum::Router> {
    Ok(crate::router::build(
        state(pool, relay(), &redis_url()).await?,
    ))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn soundcloud_track_hits_come_back_as_the_local_projection(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing)
         VALUES ('902', 'soundcloud:tracks:902', 'Local title', 'local title', 1000, 'public'),
                ('903', 'soundcloud:tracks:903', 'Private', 'private', 1000, 'private')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ($1, '901', true)",
    )
    .bind(LISTENER)
    .execute(&pool)
    .await?;
    let app = app(&pool).await?;
    let session = session(&pool).await?;

    let (status, body) = get(&app, session, "/tracks?q=projected%20tracks&limit=5").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        urns(&body),
        vec![
            "soundcloud:tracks:901".to_owned(),
            "soundcloud:tracks:902".to_owned(),
            "soundcloud:tracks:904".to_owned(),
            "soundcloud:tracks:905".to_owned(),
        ],
        "SoundCloud order, a locally private hit dropped"
    );
    assert_eq!(body["has_more"], true);
    assert_eq!(body["page_size"], 5);
    let first = &body["collection"][0];
    assert_eq!(first["user_favorite"], true);
    assert_eq!(first["access"], "playable");
    assert_eq!(first["policy"], "ALLOW");
    assert_eq!(first["favoritings_count"], 7);
    assert_eq!(first["user"]["id"], 1201);
    assert_eq!(first["_scd_meta"]["storage_state"], "pending");
    assert_eq!(body["collection"][1]["title"], "Local title");

    let stored: Vec<(String, bool)> = sqlx::query_as(
        "SELECT sc_track_id, pipeline_held FROM tracks WHERE sc_track_id LIKE '9%' ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        stored,
        vec![
            ("901".to_owned(), true),
            ("902".to_owned(), false),
            ("903".to_owned(), false),
            ("904".to_owned(), true),
            ("905".to_owned(), true),
        ],
        "only the served window is stored and only new rows are held"
    );
    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = '902'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Local title");

    let (status, body) = get(&app, session, "/tracks/soundcloud:tracks:904").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let held: bool =
        sqlx::query_scalar("SELECT pipeline_held FROM tracks WHERE sc_track_id = '904'")
            .fetch_one(&pool)
            .await?;
    assert!(!held, "opening a stored hit releases its pipeline");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn soundcloud_playlist_and_user_hits_come_back_as_the_local_projection(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_likes_playlists (user_id, playlist_urn, wanted_state)
         VALUES ($1, 'soundcloud:playlists:902', true)",
    )
    .bind(LISTENER)
    .execute(&pool)
    .await?;
    let app = app(&pool).await?;
    let session = session(&pool).await?;

    let (status, body) = get(&app, session, "/playlists?q=projected%20playlists&limit=3").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        urns(&body),
        vec![
            "soundcloud:playlists:901".to_owned(),
            "soundcloud:playlists:902".to_owned(),
            "soundcloud:playlists:903".to_owned(),
        ]
    );
    assert_eq!(body["collection"][0]["user"]["urn"], "soundcloud:users:77");
    assert_eq!(body["collection"][1]["user_favorite"], true);
    let states: i64 = sqlx::query_scalar("SELECT count(*) FROM playlist_membership_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(states, 0, "a search hit schedules no membership reconcile");

    let (status, body) = get(&app, session, "/playlists/soundcloud:playlists:901/tracks").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let due: bool = sqlx::query_scalar(
        "SELECT next_reconcile_at <= clock_timestamp() + interval '1 minute'
         FROM playlist_membership_state WHERE playlist_urn = 'soundcloud:playlists:901'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(
        due,
        "opening a stored playlist starts tracking its membership"
    );

    let (status, body) = get(&app, session, "/users?q=projected%20users&limit=2").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        urns(&body),
        vec![
            "soundcloud:users:901".to_owned(),
            "soundcloud:users:902".to_owned(),
        ]
    );
    assert_eq!(body["collection"][0]["id"], 901);
    assert_eq!(body["collection"][0]["username"], "remote 901");
    assert_eq!(body["has_more"], true);
    Ok(())
}

fn resolved_playlist(inputs: &Value) -> Option<Value> {
    inputs.get("url")?;
    Some(json!({
        "ok": true,
        "track": {
            "kind": "playlist",
            "id": 950,
            "urn": "soundcloud:playlists:950",
            "title": "Fresh mix",
            "track_count": 4,
            "sharing": "public",
            "permalink_url": "https://soundcloud.com/owner/sets/fresh-mix",
            "user": {"kind": "user", "id": 77, "urn": "soundcloud:users:77", "username": "owner"},
        },
    }))
}

async fn resolve(
    app: &axum::Router,
    session: uuid::Uuid,
    url: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let uri = format!(
        "/resolve?url={}",
        url::form_urlencoded::byte_serialize(url.as_bytes()).collect::<String>()
    );
    let response = tower::ServiceExt::oneshot(
        app.clone(),
        axum::http::Request::builder()
            .uri(uri)
            .header("x-session-id", session.to_string())
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                40000,
            ))))
            .body(axum::body::Body::empty())?,
    )
    .await?;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

async fn seed_local_track(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, permalink_url)
         VALUES ('42', 'soundcloud:tracks:42', 'Local', 'local', 1000, 'public', 'https://soundcloud.com/Artist/Song')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_pasted_link_resolves_locally_and_its_new_playlist_waits_for_membership(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_local_track(&pool).await?;
    let relay = SearchRelay::answering(Box::new(resolved_playlist), 500, json!({}));
    let app = crate::router::build(state(&pool, relay.clone(), &redis_url()).await?);
    let session = session(&pool).await?;

    let (status, body) = resolve(
        &app,
        session,
        "https://m.soundcloud.com/ARTIST/song/?in=artist/sets/mix&t=42",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["urn"], "soundcloud:tracks:42");
    assert_eq!(
        relay.lua_calls(),
        0,
        "a catalog track never reaches SoundCloud"
    );

    let (status, body) = resolve(
        &app,
        session,
        "https://soundcloud.com/owner/sets/fresh-mix?si=abc",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["urn"], "soundcloud:playlists:950");
    assert_eq!(
        relay.lua_inputs()[0]["url"],
        "https://soundcloud.com/owner/sets/fresh-mix"
    );

    let (status, body) = get(&app, session, "/playlists/soundcloud:playlists:950/tracks").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sync"]["status"], "unhydrated");
    assert_eq!(body["collection"], json!([]));
    let due: bool = sqlx::query_scalar(
        "SELECT next_reconcile_at <= clock_timestamp() + interval '1 minute'
         FROM playlist_membership_state WHERE playlist_urn = 'soundcloud:playlists:950'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(
        due,
        "opening a resolved playlist schedules its membership fetch"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_short_link_is_expanded_once_and_resolved_like_its_target(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_local_track(&pool).await?;
    let short = format!(
        "https://on.soundcloud.com/{}",
        uuid::Uuid::now_v7().simple()
    );
    let relay = SearchRelay::redirecting(
        Box::new(|_| None),
        "https://soundcloud.com/artist/song?si=abc&utm_source=tumblr&utm_medium=text",
    );
    let app = crate::router::build(state(&pool, relay.clone(), &redis_url()).await?);
    let session = session(&pool).await?;

    for _ in 0..2 {
        let (status, body) = resolve(&app, session, &format!("{short}?si=xyz")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["urn"], "soundcloud:tracks:42");
    }
    let expansions = relay
        .fetched_urls()
        .into_iter()
        .filter(|url| url.starts_with("https://on.soundcloud.com/"))
        .collect::<Vec<_>>();
    assert_eq!(
        expansions,
        vec![short],
        "expanded once, then from the cache"
    );
    assert_eq!(
        relay.lua_calls(),
        0,
        "SoundCloud is never asked to resolve a short link"
    );

    let elsewhere =
        SearchRelay::redirecting(Box::new(|_| None), "https://evil.example/artist/song");
    let app = crate::router::build(state(&pool, elsewhere, &redis_url()).await?);
    let (status, body) = resolve(
        &app,
        session,
        &format!(
            "https://on.soundcloud.com/{}",
            uuid::Uuid::now_v7().simple()
        ),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = resolve(&app, session, "https://snd.sc/abc").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    Ok(())
}
