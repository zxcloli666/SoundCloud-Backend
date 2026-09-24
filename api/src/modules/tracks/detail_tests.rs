use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use super::{TracksService, TracksServiceDependencies};
use crate::config::ColdCfg;
use crate::modules::auth::{AuthHealthService, AuthService, TokenProvider};
use crate::modules::cold_refresh::ColdRefreshService;
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::modules::sync_queue::SyncQueueService;
use crate::sc::ScClient;

struct Services {
    tracks: Arc<TracksService>,
    playlists: Arc<crate::modules::playlists::PlaylistsService>,
}

async fn services(pool: &PgPool) -> anyhow::Result<Services> {
    let deps = dependencies(pool)?;
    let nats = crate::bus::nats::NatsService::connect(
        "nats://127.0.0.1:1",
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    let background = crate::background_jobs::BackgroundJobs::new(nats);
    let indexing = crate::modules::indexing::IndexingService::new(
        pool.clone(),
        background.clone(),
        crate::background_jobs::IndexingJobs::new(background.clone()),
        420000,
    );
    deps.cold_refresh.install_indexing(indexing);
    let playlists = crate::modules::playlists::PlaylistsService::new(
        crate::modules::playlists::PlaylistsDeps {
            sc: deps.sc.clone(),
            pg: pool.clone(),
            sync_queue: deps.sync_queue.clone(),
            cold_refresh: deps.cold_refresh.clone(),
            tokens: deps.tokens.clone(),
            background_jobs: background,
        },
    );
    Ok(Services {
        tracks: TracksService::new(deps),
        playlists,
    })
}

async fn seed_private(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, sharing)
        VALUES ('42', 'soundcloud:tracks:42', 'Private title', 'private title', 120000, '17', 'private')")
        .execute(pool).await?;
    sqlx::query("INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, sharing)
        VALUES ('42', 'soundcloud:playlists:42', 'Private mix', 'private mix', '17', 'private')")
        .execute(pool).await?;
    Ok(())
}

fn remote_track() -> serde_json::Value {
    serde_json::json!({"id":42,"urn":"soundcloud:tracks:42","kind":"track","title":"Remote title","duration":120000,"sharing":"private","user":{"id":17,"username":"Owner"}})
}

fn remote_playlist() -> serde_json::Value {
    serde_json::json!({"id":42,"urn":"soundcloud:playlists:42","kind":"playlist","title":"Remote mix","sharing":"private","user":{"id":17,"username":"Owner"},"tracks":[{"id":999}]})
}

#[sqlx::test(migrations = "./migrations")]
async fn playlist_detail_with_secret_uses_local_public_and_owner_records(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_private(&pool).await?;
    let services = services(&pool).await?;
    let params = [
        ("secret_token".into(), "s-unneeded".into()),
        ("show_tracks".into(), "true".into()),
    ];
    let owner = tokio::time::timeout(
        Duration::from_secs(2),
        services
            .playlists
            .get_by_id(Uuid::nil(), "17", "42", &params),
    )
    .await??;
    assert_eq!(owner["title"], "Private mix");
    sqlx::query("UPDATE playlists SET sharing = 'public'")
        .execute(&pool)
        .await?;
    let public = tokio::time::timeout(
        Duration::from_secs(2),
        services
            .playlists
            .get_by_id(Uuid::nil(), "18", "42", &params),
    )
    .await??;
    assert_eq!(public["title"], "Private mix");
    assert!(public.get("tracks").is_none());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn secret_observations_return_persisted_metadata_and_do_not_persist_access(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_private(&pool).await?;
    let services = services(&pool).await?;
    let track = services
        .tracks
        .get_by_id_with_fetch("18", "42", true, || async { Ok(remote_track()) })
        .await?;
    let playlist = services
        .playlists
        .get_by_id_with_fetch("18", "42", true, || async { Ok(remote_playlist()) })
        .await?;
    assert_eq!(track["title"], "Remote title");
    assert_eq!(playlist["title"], "Remote mix");
    assert!(playlist.get("tracks").is_none());
    let stored_track: String =
        sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    let stored_playlist: String =
        sqlx::query_scalar("SELECT title FROM playlists WHERE sc_playlist_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        (stored_track.as_str(), stored_playlist.as_str()),
        ("Remote title", "Remote mix")
    );
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let track_denied = services
        .tracks
        .get_by_id_with_fetch("18", "42", false, || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(remote_track())
        })
        .await;
    let playlist_denied = services
        .playlists
        .get_by_id_with_fetch("18", "42", false, || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(remote_playlist())
        })
        .await;
    assert_eq!(
        track_denied.unwrap_err().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(
        playlist_denied.unwrap_err().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    let rejected = services
        .tracks
        .get_by_id_with_fetch("18", "42", true, || async {
            Err(crate::error::AppError::not_found("Secret rejected"))
        })
        .await;
    assert_eq!(
        rejected.unwrap_err().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn metadata_changed_during_secret_verification_wins_over_the_remote_reply(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_private(&pool).await?;
    let services = services(&pool).await?;
    let track = services
        .tracks
        .get_by_id_with_fetch("18", "42", true, || async {
            services
                .tracks
                .update(
                    "17",
                    "42",
                    &serde_json::json!({"track":{"title":"Desired title"}}),
                )
                .await?;
            Ok(remote_track())
        })
        .await?;
    let playlist = services
        .playlists
        .get_by_id_with_fetch("18", "42", true, || async {
            services
                .playlists
                .update(
                    "17",
                    "42",
                    &serde_json::json!({"playlist":{"title":"Desired mix"}}),
                    false,
                    Uuid::now_v7(),
                )
                .await?;
            Ok(remote_playlist())
        })
        .await?;
    assert_eq!(track["title"], "Desired title");
    assert_eq!(playlist["title"], "Desired mix");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn deletion_during_secret_verification_never_returns_the_remote_entity(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_private(&pool).await?;
    let services = services(&pool).await?;
    let track = services
        .tracks
        .get_by_id_with_fetch("18", "42", true, || async {
            services.tracks.delete("17", "42").await?;
            Ok(remote_track())
        })
        .await;
    let playlist = services
        .playlists
        .get_by_id_with_fetch("18", "42", true, || async {
            services.playlists.delete("17", "42").await?;
            Ok(remote_playlist())
        })
        .await;
    assert_eq!(
        track.unwrap_err().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(
        playlist.unwrap_err().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn secret_response_identity_must_match_the_requested_entity(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_private(&pool).await?;
    let services = services(&pool).await?;
    let result = services
        .tracks
        .get_by_id_with_fetch("18", "42", true, || async {
            let mut wrong = remote_track();
            wrong["id"] = serde_json::json!(43);
            Ok(wrong)
        })
        .await;
    assert_eq!(
        result.unwrap_err().public_code(),
        "invalid_catalog_response"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tracks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    Ok(())
}

fn dependencies(pool: &PgPool) -> anyhow::Result<TracksServiceDependencies> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: "http://127.0.0.1:1".into(),
        proxy_fallback: false,
        api_base: None,
        home_base: None,
    })?;
    let auth = AuthService::new(
        pool.clone(),
        sc.clone(),
        OAuthAppsService::new(pool.clone()),
        AuthHealthService::with_database(redis.clone(), pool.clone()),
    );
    let tokens = TokenProvider::new(auth, OAuthAppTokenService::new(pool.clone()));
    Ok(TracksServiceDependencies {
        sc,
        pg: pool.clone(),
        sync_queue: SyncQueueService::new(pool.clone(), redis),
        tokens,
        cold_refresh: ColdRefreshService::new(
            pool.clone(),
            ColdCfg {
                track_ttl_sec: 3600,
                user_ttl_sec: 3600,
                playlist_ttl_sec: 3600,
                liked_tracks_ttl_sec: 3600,
                liked_playlists_ttl_sec: 3600,
                followings_ttl_sec: 3600,
                owned_ttl_sec: 3600,
                evict_after_sec: 86400,
            },
        ),
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn public_detail_with_secret_works_without_any_soundcloud_connection(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
        VALUES ('42', 'soundcloud:tracks:42', 'Local title', 'local title', 120000, '17')")
        .execute(&pool).await?;
    let service = TracksService::new(dependencies(&pool)?);
    let value = tokio::time::timeout(
        Duration::from_secs(2),
        service.get_by_id(
            Uuid::nil(),
            "18",
            "42",
            &[("secret_token".into(), "s-unneeded".into())],
        ),
    )
    .await??;
    assert_eq!(value["title"], "Local title");
    assert_eq!(value["urn"], "soundcloud:tracks:42");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn private_owner_detail_with_secret_works_without_any_soundcloud_connection(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, sharing)
        VALUES ('42', 'soundcloud:tracks:42', 'Private title', 'private title', 120000, '17', 'private')")
        .execute(&pool).await?;
    let service = TracksService::new(dependencies(&pool)?);
    let value = tokio::time::timeout(
        Duration::from_secs(2),
        service.get_by_id(
            Uuid::nil(),
            "17",
            "soundcloud:tracks:42",
            &[("secret_token".into(), "s-unneeded".into())],
        ),
    )
    .await??;
    assert_eq!(value["title"], "Private title");
    assert_eq!(value["sharing"], "private");
    Ok(())
}

async fn pending_jobs(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT dedup_key FROM background_jobs WHERE kind = 'catalog.refresh' ORDER BY dedup_key",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| row.0).collect())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_track_is_queued_for_refresh_instead_of_fetched_in_the_request(
    pool: PgPool,
) -> anyhow::Result<()> {
    let services = services(&pool).await?;
    let calls = std::sync::atomic::AtomicUsize::new(0);

    let error = services
        .tracks
        .get_by_id_with_fetch("18", "42", false, || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(remote_track())
        })
        .await
        .unwrap_err();

    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        pending_jobs(&pool).await?,
        vec!["track:42:public".to_owned()]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_playlist_is_queued_for_refresh_instead_of_fetched_in_the_request(
    pool: PgPool,
) -> anyhow::Result<()> {
    let services = services(&pool).await?;
    let calls = std::sync::atomic::AtomicUsize::new(0);

    let error = services
        .playlists
        .get_by_id_with_fetch("18", "42", false, || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(remote_playlist())
        })
        .await
        .unwrap_err();

    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        pending_jobs(&pool).await?,
        vec!["playlist:42:public".to_owned()]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_private_track_of_another_user_is_not_probed_against_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_private(&pool).await?;
    let services = services(&pool).await?;
    let calls = std::sync::atomic::AtomicUsize::new(0);

    let error = services
        .tracks
        .get_by_id_with_fetch("18", "42", false, || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(remote_track())
        })
        .await
        .unwrap_err();

    assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(pending_jobs(&pool).await?.is_empty());
    Ok(())
}
