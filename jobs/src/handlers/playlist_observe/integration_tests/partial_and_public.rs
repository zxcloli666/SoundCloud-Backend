use std::sync::Arc;

use backend_contracts::PlaylistObservePayload;
use serde_json::json;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

use super::{
    ScriptedServer, install_schema, membership_mutations, membership_of, observe_handler,
    projection_of, read_request, reduce_with, seed_rebased_playlist, snapshot_of,
    write_json_response,
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

async fn serve_foreign_playlist() -> anyhow::Result<ScriptedServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let api_url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let metadata = json!({
        "id": 404,
        "user": { "id": 999 },
        "track_count": 3,
        "last_modified": "2026-09-30T10:00:00Z"
    })
    .to_string();
    let track = |id: u64| {
        json!({
            "id": id,
            "title": format!("Track {id}"),
            "duration": 120_000,
            "sharing": "public",
            "user": { "id": 999, "username": "artist" }
        })
    };
    let tracks = json!({
        "collection": [track(11), track(12), track(11)],
        "next_href": null
    })
    .to_string();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        for body in [metadata.clone(), tracks, metadata] {
            let (mut stream, _) = listener.accept().await?;
            seen.lock().await.push(read_request(&mut stream).await?);
            write_json_response(&mut stream, &body).await?;
        }
        Ok(())
    });
    Ok((api_url, requests, server))
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

#[sqlx::test(migrations = false)]
async fn a_public_playlist_of_a_stranger_is_read_with_an_app_token_and_never_written_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "public").await?;
    let (api_url, requests, server) = serve_foreign_playlist().await?;

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
    let (status, error) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT sync_status, last_error FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(FOREIGN)
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "clean");
    assert_eq!(error, None);
    let projection = sqlx::query_scalar::<_, String>(
        "SELECT sc_track_id FROM playlist_track_projection
         WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(FOREIGN)
    .fetch_all(&pool)
    .await?;
    assert_eq!(projection, ["11", "12"]);
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
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_private_playlist_of_a_stranger_still_waits_for_its_owner(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_foreign_playlist(&pool, "private").await?;
    let (api_url, requests, server) = serve_foreign_playlist().await?;

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
