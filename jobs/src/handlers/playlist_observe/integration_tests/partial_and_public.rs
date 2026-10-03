use std::sync::Arc;

use backend_contracts::PlaylistObservePayload;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

use super::{
    ScriptedServer, install_schema, membership_mutations, membership_of, observe_handler,
    projection_of, read_request, reduce_with, seed_rebased_playlist, snapshot_of,
};

const FOREIGN: &str = "soundcloud:playlists:404";

async fn seed_foreign_playlist(pool: &PgPool, sharing: &str) -> anyhow::Result<()> {
    let oauth_app_id = Uuid::from_u128(1);
    sqlx::query(
        "INSERT INTO oauth_apps (id, client_id, client_secret)
         VALUES ($1, 'client-id', 'client-secret')",
    )
    .bind(oauth_app_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, access_token, expires_at)
         VALUES ($1, 'app-access', now() + interval '1 day')",
    )
    .bind(oauth_app_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlists (urn, owner_sc_user_id, sharing, track_count)
         VALUES ($1, '999', $2, 3)",
    )
    .bind(FOREIGN)
    .bind(sharing)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn, next_reconcile_at)
         VALUES ($1, clock_timestamp())",
    )
    .bind(FOREIGN)
    .execute(pool)
    .await?;
    Ok(())
}

fn metadata() -> String {
    json!({
        "id": 404,
        "user": { "id": 999 },
        "track_count": 3,
        "last_modified": "2026-09-30T10:00:00Z"
    })
    .to_string()
}

fn track(id: u64) -> Value {
    json!({
        "id": id,
        "title": format!("Track {id}"),
        "duration": 120_000,
        "sharing": "public",
        "user": { "id": 999, "username": "artist" }
    })
}

fn tracks(collection: Vec<Value>) -> String {
    json!({ "collection": collection, "next_href": null }).to_string()
}

async fn serve_foreign(responses: Vec<(&'static str, String)>) -> anyhow::Result<ScriptedServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let api_url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().await?;
            seen.lock().await.push(read_request(&mut stream).await?);
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(body.as_bytes()).await?;
            stream.shutdown().await?;
        }
        Ok(())
    });
    Ok((api_url, requests, server))
}

async fn serve_foreign_playlist(collection: Vec<Value>) -> anyhow::Result<ScriptedServer> {
    serve_foreign(vec![
        ("200 OK", metadata()),
        ("200 OK", tracks(collection)),
        ("200 OK", metadata()),
    ])
    .await
}

async fn observe_foreign(pool: &PgPool, api_url: Url) -> anyhow::Result<()> {
    observe_handler(pool, api_url)?
        .observe(
            Uuid::now_v7(),
            1,
            PlaylistObservePayload {
                playlist_urn: FOREIGN.to_owned(),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn foreign_state(pool: &PgPool) -> anyhow::Result<(String, Option<String>)> {
    Ok(sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT sync_status, last_error FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(FOREIGN)
    .fetch_one(pool)
    .await?)
}

async fn foreign_due_in(pool: &PgPool) -> anyhow::Result<chrono::Duration> {
    let due = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT next_reconcile_at FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(FOREIGN)
    .fetch_one(pool)
    .await?;
    Ok(due - chrono::Utc::now())
}

async fn foreign_projection(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT sc_track_id FROM playlist_track_projection
         WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(FOREIGN)
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn a_public_playlist_of_a_stranger_is_read_with_an_app_token_and_never_written_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "public").await?;
    let (api_url, requests, server) =
        serve_foreign_playlist(vec![track(11), track(12), track(11)]).await?;

    observe_foreign(&pool, api_url).await?;
    server.await??;

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.contains("OAuth app-access")),
        "a stranger's playlist must be read with the app token"
    );
    assert_eq!(foreign_state(&pool).await?, ("clean".to_owned(), None));
    assert_eq!(foreign_projection(&pool).await?, ["11", "12"]);
    let observation = sqlx::query_as::<_, (String, String, i32, i32, bool)>(
        "SELECT authority, outcome, declared_track_count, observed_track_count, write_eligible
         FROM playlist_remote_observations WHERE playlist_urn = $1",
    )
    .bind(FOREIGN)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        observation,
        ("public".to_owned(), "incomplete".to_owned(), 3, 2, false)
    );
    assert!(
        foreign_due_in(&pool).await? > chrono::Duration::minutes(55),
        "a stranger's clean playlist was due on the shared app tokens within the hour"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_private_playlist_of_a_stranger_still_waits_for_its_owner(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "private").await?;
    let (api_url, requests, server) = serve_foreign_playlist(vec![track(11)]).await?;

    observe_foreign(&pool, api_url).await?;
    server.abort();

    assert!(
        requests.lock().await.is_empty(),
        "a private playlist was read without its owner"
    );
    let status = sqlx::query_scalar::<_, String>(
        "SELECT sync_status FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(FOREIGN)
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "auth_required");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_public_playlist_whose_owner_revoked_the_app_falls_back_to_an_app_token(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "public").await?;
    let revoked_app = Uuid::from_u128(2);
    sqlx::query(
        "INSERT INTO oauth_apps (id, client_id, client_secret, active)
         VALUES ($1, 'revoked-client', 'revoked-secret', false)",
    )
    .bind(revoked_app)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
         ) VALUES (
             $1, '999', $2, 'owner-access', 'owner-refresh', now() + interval '1 day', ''
         )",
    )
    .bind(Uuid::now_v7())
    .bind(revoked_app)
    .execute(&pool)
    .await?;
    let (api_url, requests, server) = serve_foreign(vec![
        ("401 Unauthorized", "{}".to_owned()),
        ("200 OK", metadata()),
        ("200 OK", tracks(vec![track(11), track(12), track(13)])),
        ("200 OK", metadata()),
    ])
    .await?;

    observe_foreign(&pool, api_url).await?;
    server.await??;

    let requests = requests.lock().await.clone();
    assert!(requests[0].contains("OAuth owner-access"));
    assert!(
        requests[1..]
            .iter()
            .all(|request| request.contains("OAuth app-access")),
        "a revoked owner must not leave a public playlist unread"
    );
    assert_eq!(foreign_state(&pool).await?, ("clean".to_owned(), None));
    assert_eq!(foreign_projection(&pool).await?, ["11", "12", "13"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_empty_read_of_a_non_empty_playlist_keeps_the_shown_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "public").await?;
    sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ($1, 0, '11')",
    )
    .bind(FOREIGN)
    .execute(&pool)
    .await?;
    let (api_url, _, server) = serve_foreign_playlist(Vec::new()).await?;

    observe_foreign(&pool, api_url).await?;
    server.abort();

    assert_eq!(
        foreign_state(&pool).await?,
        (
            "retry_wait".to_owned(),
            Some("soundcloud_playlist_malformed".to_owned())
        )
    );
    assert_eq!(foreign_projection(&pool).await?, ["11"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_empty_read_of_a_never_shown_playlist_settles_with_nothing_available(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "public").await?;
    let (api_url, _, server) = serve_foreign_playlist(Vec::new()).await?;

    observe_foreign(&pool, api_url).await?;
    server.await??;

    assert_eq!(foreign_state(&pool).await?, ("clean".to_owned(), None));
    assert!(foreign_projection(&pool).await?.is_empty());
    let observation = sqlx::query_as::<_, (String, i32, i32)>(
        "SELECT outcome, declared_track_count, observed_track_count
         FROM playlist_remote_observations WHERE playlist_urn = $1",
    )
    .bind(FOREIGN)
    .fetch_one(&pool)
    .await?;
    assert_eq!(observation, ("incomplete".to_owned(), 3, 0));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_short_owner_read_is_shown_but_never_offered_to_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_rebased_playlist(&pool).await?;
    let mut short = snapshot_of(&["1", "7", "2"]);
    short.track_count = 4;

    reduce_with(&pool, &short, true).await?;

    assert_eq!(projection_of(&pool).await?, vec!["1", "7", "2", "9"]);
    assert_eq!(membership_of(&pool).await?.0, "shadow_ready");
    assert!(
        membership_mutations(&pool).await?.is_empty(),
        "a full-list write built from a short read would delete hidden tracks"
    );
    Ok(())
}
