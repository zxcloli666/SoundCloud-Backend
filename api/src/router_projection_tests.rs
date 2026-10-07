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
