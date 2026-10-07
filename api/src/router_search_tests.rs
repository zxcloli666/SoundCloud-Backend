use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::bus::nats::NatsService;
use crate::cache::CacheService;
use crate::common::admission::PublicAdmission;
use crate::config::{
    AdminCfg, AdmissionCfg, AdmissionLimitCfg, AppConfig, ColdCfg, CollabTriggerCfg, DatabaseCfg,
    DbSslCfg, NatsCfg, QdrantCfg, RedisCfg, SoundcloudCfg, SoundwaveCfg, StorageCfg, StreamingCfg,
    SubscriptionsCfg,
};
use crate::modules::auth::{AuthHealthService, AuthService, LinkService, TokenProvider};
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::sc::read_tests::SearchRelay;
use crate::sc::{ScClient, ScReadService};
use crate::state::AppState;

pub(super) const LISTENER: &str = "17";

pub(super) fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

fn config(redis: &str) -> AppConfig {
    let limit = AdmissionLimitCfg {
        per_client: 1_000,
        global: 1_000_000,
    };
    AppConfig {
        port: 0,
        soundcloud: SoundcloudCfg {
            proxy_url: String::new(),
            proxy_fallback: false,
        },
        database: DatabaseCfg {
            url: String::new(),
            ssl: DbSslCfg {
                mode: None,
                root_cert: None,
                client_cert: None,
                client_key: None,
            },
            pool_max: 4,
            acquire_timeout: Duration::from_secs(5),
        },
        streaming: StreamingCfg {
            service_url: url::Url::parse("http://127.0.0.1:1").expect("a streaming url"),
            ticket_key: stream_ticket::StreamTicketKey::from_base64(
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .expect("a ticket key"),
        },
        admin: AdminCfg {
            token: String::new().into(),
        },
        redis: RedisCfg {
            url: redis.to_owned(),
        },
        admission: AdmissionCfg {
            window: Duration::from_secs(60),
            timeout: Duration::from_millis(500),
            max_in_flight: 64,
            login: limit,
            link_create: limit,
            resolve: limit,
            sc_search: limit,
            catalog_miss: limit,
        },
        nats: NatsCfg {
            url: "nats://127.0.0.1:1".to_owned(),
        },
        qdrant: QdrantCfg {
            grpc_url: "http://127.0.0.1:1".to_owned(),
            api_key: String::new().into(),
        },
        storage: StorageCfg { url: String::new() },
        subscriptions: SubscriptionsCfg {
            always_premium: true,
        },
        soundwave: SoundwaveCfg {
            popularity_boost: 0.0,
            artist_cap: 2,
        },
        collab_trigger: CollabTriggerCfg {
            event_threshold: 100,
            cooldown: Duration::from_secs(600),
        },
        cold: ColdCfg {
            track_ttl_sec: 3600,
            user_ttl_sec: 3600,
            playlist_ttl_sec: 3600,
            liked_tracks_ttl_sec: 3600,
            liked_playlists_ttl_sec: 3600,
            followings_ttl_sec: 3600,
            owned_ttl_sec: 300,
            evict_after_sec: 86_400,
        },
        max_track_duration_ms: 420_000,
        premium_reserve: false,
        reserve_backend: false,
    }
}

pub(super) async fn state(
    pg: &PgPool,
    relay: Arc<SearchRelay>,
    redis: &str,
) -> anyhow::Result<AppState> {
    let config = Arc::new(config(redis));
    let pg = pg.clone();
    let redis_pool = deadpool_redis::Config::from_url(redis)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let admission = PublicAdmission::new(redis_pool.clone(), config.admission.clone());
    let nats =
        NatsService::connect(&config.nats.url, tokio_util::sync::CancellationToken::new()).await?;
    let qdrant = crate::qdrant::QdrantService::connect(&config.qdrant)?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: String::new(),
        proxy_fallback: false,
        api_base: Some("http://127.0.0.1:2".to_owned()),
        home_base: Some("http://127.0.0.1:1".to_owned()),
    })?
    .with_relay(relay);
    let oauth_apps = OAuthAppsService::new(pg.clone());
    let auth = AuthService::new(
        pg.clone(),
        sc.clone(),
        oauth_apps.clone(),
        AuthHealthService::with_database(redis_pool.clone(), pg.clone()),
    );
    let link = LinkService::new(pg.clone(), auth.clone());
    let tokens = TokenProvider::new(auth.clone(), OAuthAppTokenService::new(pg.clone()));
    let resolve = ScReadService::new(sc.clone(), tokens.clone(), pg.clone());
    let cache = CacheService::new(redis_pool.clone());
    let background_jobs = crate::background_jobs::BackgroundJobs::new(nats.clone());
    let indexing_jobs = crate::background_jobs::IndexingJobs::new(background_jobs.clone());
    let collab_jobs = crate::background_jobs::CollabJobs::new(
        background_jobs.clone(),
        redis_pool.clone(),
        &config.collab_trigger,
    );
    let events = crate::modules::events::EventsService::new(
        pg.clone(),
        background_jobs.clone(),
        indexing_jobs.clone(),
        collab_jobs.clone(),
    );
    let subscriptions = crate::modules::subscriptions::SubscriptionsService::new(pg.clone(), true);
    let auras = crate::modules::auras::AurasService::new(pg.clone(), subscriptions.clone());
    let sync_queue =
        crate::modules::sync_queue::SyncQueueService::new(pg.clone(), redis_pool.clone());
    let cold_refresh =
        crate::modules::cold_refresh::ColdRefreshService::new(pg.clone(), config.cold.clone());
    let me =
        crate::modules::me::MeService::new(pg.clone(), sync_queue.clone(), cold_refresh.clone());
    let s3 = crate::modules::recommendations::S3VerifierService::new(
        wreq::Client::new(),
        String::new(),
        pg.clone(),
    );
    let worker =
        crate::modules::lyrics::WorkerClient::new(nats.clone(), cache.clone(), qdrant.clone());
    let collab_vector = crate::modules::collab::CollabVectorService::new(qdrant.clone());
    let recommendations = crate::modules::recommendations::RecommendationsService::new(
        qdrant,
        pg.clone(),
        nats,
        redis_pool,
        worker,
        s3,
        collab_vector.clone(),
        config.soundwave.clone(),
    );
    let indexing = crate::modules::indexing::IndexingService::new(
        pg.clone(),
        background_jobs.clone(),
        indexing_jobs,
        config.max_track_duration_ms,
    );
    cold_refresh.install_indexing(indexing.clone());
    let miss = crate::modules::resolve::CatalogMiss::new(
        pg.clone(),
        resolve.clone(),
        indexing.clone(),
        admission.clone(),
    );
    let tracks = crate::modules::tracks::TracksService::new(
        crate::modules::tracks::TracksServiceDependencies {
            sc: sc.clone(),
            pg: pg.clone(),
            sync_queue: sync_queue.clone(),
            cold_refresh: cold_refresh.clone(),
            tokens: tokens.clone(),
            miss: miss.clone(),
        },
    );
    let playlists = crate::modules::playlists::PlaylistsService::new(
        crate::modules::playlists::PlaylistsDeps {
            sc,
            pg: pg.clone(),
            sync_queue: sync_queue.clone(),
            cold_refresh: cold_refresh.clone(),
            tokens,
            background_jobs: background_jobs.clone(),
            miss: miss.clone(),
        },
    );
    let users = crate::modules::users::UsersService::new(pg.clone(), cold_refresh, miss.clone());
    let dislikes = crate::modules::dislikes::DislikesService::new(pg.clone(), events.clone());
    let likes = crate::modules::likes::LikesService::new(
        pg.clone(),
        sync_queue.clone(),
        indexing.clone(),
        events.clone(),
    );
    events.install_dislikes(dislikes.clone());
    Ok(AppState {
        http_metrics: Arc::new(crate::common::http_metrics::HttpMetrics::new()),
        search: crate::modules::search::SearchService::new(pg.clone(), cache.clone()),
        soundcloud_search: crate::modules::soundcloud_search::SoundCloudSearch::new(
            resolve.clone(),
            cache.clone(),
            admission.clone(),
        ),
        vibe: crate::modules::search::VibeSearchService::new(
            pg.clone(),
            cache.clone(),
            recommendations.clone(),
        ),
        history: crate::modules::history::HistoryService::new(pg.clone()),
        featured: crate::modules::featured::FeaturedService::new(pg.clone()),
        lyrics: crate::modules::lyrics::LyricsService::new(
            pg.clone(),
            background_jobs.clone(),
            false,
        ),
        discover: crate::modules::discover::DiscoverService::new(pg.clone(), cache.clone()),
        config,
        background_jobs,
        cache,
        auth,
        admission,
        link,
        oauth_apps,
        events,
        dislikes,
        subscriptions,
        auras,
        me,
        tracks,
        playlists,
        users,
        likes,
        resolve,
        miss,
        collab_vector,
        collab_jobs,
        indexing,
        recommendations,
        sync_queue,
        pg,
    })
}

pub(super) async fn session(pool: &PgPool) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, 'Listener', 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(LISTENER)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

pub(super) async fn get(
    app: &axum::Router,
    session: Uuid,
    uri: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("x-session-id", session.to_string())
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), 4 * 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

pub(super) fn soundcloud_hits() -> Arc<SearchRelay> {
    SearchRelay::answering(
        Box::new(|inputs| {
            let kind = match inputs["type"].as_str() {
                Some("users") => "users",
                Some("tracks") => "tracks",
                _ => "playlists",
            };
            let collection: Vec<Value> = (1..=20)
                .map(|id| {
                    json!({
                        "id": 900 + id,
                        "urn": format!("soundcloud:{kind}:{}", 900 + id),
                        "kind": kind.trim_end_matches('s'),
                        "title": format!("remote {id}"),
                        "username": format!("remote {id}"),
                    })
                })
                .collect();
            Some(json!({"ok": true, "collection": collection, "next_href": null}))
        }),
        500,
        json!({}),
    )
}

pub(super) async fn app(pool: &PgPool, redis: &str) -> anyhow::Result<axum::Router> {
    Ok(crate::router::build(
        state(pool, soundcloud_hits(), redis).await?,
    ))
}

pub(super) async fn seed_tracks(pool: &PgPool, ids: &[&str]) -> anyhow::Result<()> {
    for id in ids {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist,
                                 uploader_username, uploader_sc_user_id, duration_ms, play_count_sc, sharing)
             VALUES ($1, 'soundcloud:tracks:' || $1, 'Midnight Train ' || $1, 'midnight train ' || $1,
                     'Night Owls', 'Night Owls', '555', 200000, 1000, 'public')",
        )
        .bind(id)
        .execute(pool)
        .await?;
    }
    sqlx::query("REFRESH MATERIALIZED VIEW search_terms")
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) fn urns(body: &Value) -> Vec<String> {
    body["collection"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["urn"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_import_contract_answers_with_soundcloud_hits(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session(&pool).await?;
    let (status, body) = get(
        &app,
        session,
        "/tracks?q=midnight%20train&limit=3&linked_partitioning=true",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["collection"][0]["urn"], "soundcloud:tracks:901");
    assert_eq!(urns(&body).len(), 3);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_query_wins_over_ids(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5"]).await?;
    let app = app(&pool, &redis_url()).await?;
    let session = session(&pool).await?;
    let (status, body) = get(&app, session, "/tracks?q=midnight&ids=5&limit=5").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!urns(&body).contains(&"soundcloud:tracks:5".to_owned()));
    assert_eq!(body["collection"][0]["urn"], "soundcloud:tracks:901");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn ids_are_read_locally_in_request_order(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5", "7"]).await?;
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let session = session(&pool).await?;
    let (status, body) = get(&app, session, "/tracks?ids=7,soundcloud:tracks:5,404").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        urns(&body),
        vec![
            "soundcloud:tracks:7".to_owned(),
            "soundcloud:tracks:5".to_owned()
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn no_params_is_an_empty_page(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let session = session(&pool).await?;
    for uri in ["/tracks", "/playlists", "/users"] {
        let (status, body) = get(&app, session, uri).await?;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["collection"], json!([]), "{uri}");
        assert_eq!(body["has_more"], false, "{uri}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn playlist_and_user_search_accept_every_released_param(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session(&pool).await?;
    let (status, body) = get(
        &app,
        session,
        "/playlists?q=night&show_tracks=true&access=playable&linked_partitioning=true&limit=10",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["collection"][0]["urn"], "soundcloud:playlists:901");
    let (status, body) = get(
        &app,
        session,
        "/users?q=night&linked_partitioning=true&limit=10",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["collection"][0]["urn"], "soundcloud:users:901");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn catalog_search_ignores_legacy_params(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5"]).await?;
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let session = session(&pool).await?;
    for path in ["/search/db/tracks", "/search/db/playlists"] {
        let uri = format!(
            "{path}?q=midnight&access=playable&genres=rock,pop&ids=1,2&linked_partitioning=true&limit=5"
        );
        let (status, body) = get(&app, session, &uri).await?;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn catalog_tracks_carry_the_listener_like(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5", "7"]).await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ($1, '5', true)",
    )
    .bind(LISTENER)
    .execute(&pool)
    .await?;
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let session = session(&pool).await?;
    let (status, body) = get(
        &app,
        session,
        "/search/db/tracks?q=midnight%20train&limit=5",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let liked: Vec<(String, bool)> = body["collection"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|track| {
            Some((
                track["urn"].as_str()?.to_owned(),
                track["user_favorite"].as_bool().unwrap_or(false),
            ))
        })
        .collect();
    assert!(
        liked.contains(&("soundcloud:tracks:5".to_owned(), true)),
        "{liked:?}"
    );
    assert!(
        liked.contains(&("soundcloud:tracks:7".to_owned(), false)),
        "{liked:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn catalog_search_pages_twenty_without_a_limit(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["5"]).await?;
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let session = session(&pool).await?;
    for path in [
        "/search/db/tracks",
        "/search/db/playlists",
        "/search/db/users",
        "/search/db/artists",
        "/search/db/albums",
        "/search/lyrics",
    ] {
        let uri = format!("{path}?q=midnight");
        let (status, body) = get(&app, session, &uri).await?;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["page_size"], 20, "{uri}");
    }
    Ok(())
}
