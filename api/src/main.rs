#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod background_jobs;
#[cfg(test)]
#[path = "background_surface_tests.rs"]
mod background_surface_tests;
mod bus;
mod cache;
#[cfg(test)]
#[path = "comment_surface_tests.rs"]
mod comment_surface_tests;
mod common;
mod config;
#[cfg(test)]
mod contract_snapshots;
mod db;
#[cfg(test)]
#[path = "env_surface_tests.rs"]
mod env_surface_tests;
mod error;
mod metrics;
mod modules;
#[cfg(feature = "profiling")]
mod profiling;
mod qdrant;
#[cfg(test)]
#[path = "query_surface_tests.rs"]
mod query_surface_tests;
mod redis;
mod router;
mod sc;
#[cfg(test)]
#[path = "sc_surface_tests.rs"]
mod sc_surface_tests;
#[cfg(test)]
#[path = "secret_surface_tests.rs"]
mod secret_surface_tests;
mod state;
mod telemetry;

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::bus::nats::NatsService;
use crate::cache::CacheService;
use crate::common::admission::PublicAdmission;
use crate::config::AppConfig;
use crate::modules::auras::AurasService;
use crate::modules::auth::{AuthService, LinkService, TokenProvider};
use crate::modules::cold_refresh::ColdRefreshService;
use crate::modules::collab::CollabVectorService;
use crate::modules::discover::DiscoverService;
use crate::modules::dislikes::DislikesService;
use crate::modules::events::EventsService;
use crate::modules::featured::FeaturedService;
use crate::modules::history::HistoryService;
use crate::modules::indexing::IndexingService;
use crate::modules::likes::LikesService;
use crate::modules::lyrics::{LyricsService, WorkerClient};
use crate::modules::me::MeService;
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::modules::playlists::{PlaylistsDeps, PlaylistsService};
use crate::modules::recommendations::{RecommendationsService, S3VerifierService};
use crate::modules::search::SearchService;
use crate::modules::subscriptions::SubscriptionsService;
use crate::modules::sync_queue::SyncQueueService;
use crate::modules::tracks::TracksService;
use crate::modules::users::UsersService;
use crate::qdrant::QdrantService;
use crate::sc::{ScClient, ScReadService};
use crate::state::AppState;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tls_common::init_crypto();
    telemetry::init();
    metrics::init();

    let config = Arc::new(AppConfig::from_env());
    info!(port = config.port, "backend starting");
    let reserve = config.is_reserve();
    if reserve {
        info!(
            premium_reserve = config.premium_reserve,
            reserve_backend = config.reserve_backend,
            "reserve mode ON: background pipelines disabled"
        );
    }

    let pg = db::connect(&config)
        .await
        .expect("Failed to connect to PostgreSQL");
    db::verify_schema(&pg)
        .await
        .expect("PostgreSQL schema is incompatible; run jobs-migrate core before API");
    info!("PostgreSQL connected");

    let redis_pool = redis::connect(&config).expect("Failed to create Redis pool");
    let admission = PublicAdmission::new(
        redis::connect_admission(&config).expect("Failed to create admission Redis pool"),
        config.admission.clone(),
    );
    info!("Redis pool ready");

    let shutdown = CancellationToken::new();

    let nats = NatsService::connect(&config.nats.url, shutdown.clone())
        .await
        .expect("Failed to connect to NATS");
    info!("NATS connected");

    let qdrant = match QdrantService::connect(&config.qdrant) {
        Ok(qdrant) => qdrant,
        Err(error) => {
            error!(%error, "Qdrant client initialization failed");
            std::process::exit(1);
        }
    };
    if let Err(error) = qdrant.prepare_required_collections().await {
        error!(%error, "Qdrant startup validation failed");
        std::process::exit(1);
    }
    info!("Qdrant ready");

    let http_client = sc_fingerprint::builder(None)
        .tcp_keepalive(Duration::from_secs(60))
        .pool_max_idle_per_host(20)
        .pool_idle_timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build shared HTTP client");

    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: config.soundcloud.proxy_url.clone(),
        proxy_fallback: config.soundcloud.proxy_fallback,
        api_base: None,
        home_base: None,
    })
    .expect("Failed to build SC HTTP client");

    let relay_client = build_call_relay("backend").await;
    let sc = match relay_client.clone() {
        Some(r) => sc.with_relay(r),
        None => sc,
    };
    let oauth_apps = OAuthAppsService::new(pg.clone());
    match oauth_apps.count_active().await {
        Ok(n) => info!(active = n, "Active OAuth apps"),
        Err(e) => warn!(error = %e, "Failed to count active OAuth apps"),
    }

    let auth_health =
        crate::modules::auth::AuthHealthService::with_database(redis_pool.clone(), pg.clone());
    let auth = AuthService::new(pg.clone(), sc.clone(), oauth_apps.clone(), auth_health);
    let link = LinkService::new(pg.clone(), auth.clone());

    let oauth_app_tokens = OAuthAppTokenService::new(pg.clone());
    let tokens = TokenProvider::new(auth.clone(), oauth_app_tokens.clone());
    let resolve = ScReadService::new(sc.clone(), tokens.clone(), pg.clone());

    let cache = CacheService::new(redis_pool.clone());
    let background_jobs = crate::background_jobs::BackgroundJobs::new(nats.clone());
    let indexing_jobs = crate::background_jobs::IndexingJobs::new(background_jobs.clone());
    let collab_jobs = crate::background_jobs::CollabJobs::new(
        background_jobs.clone(),
        redis_pool.clone(),
        &config.collab_trigger,
    );
    let events = EventsService::new(
        pg.clone(),
        background_jobs.clone(),
        indexing_jobs.clone(),
        collab_jobs.clone(),
    );
    let subscriptions = SubscriptionsService::new(pg.clone(), config.subscriptions.always_premium);
    let auras = AurasService::new(pg.clone(), subscriptions.clone());
    let sync_queue = SyncQueueService::new(pg.clone(), redis_pool.clone());
    let cold_refresh = ColdRefreshService::new(pg.clone(), config.cold.clone());
    let me = MeService::new(pg.clone(), sync_queue.clone(), cold_refresh.clone());
    let s3_verifier =
        S3VerifierService::new(http_client.clone(), config.storage.url.clone(), pg.clone());
    let worker = WorkerClient::new(nats.clone(), cache.clone(), qdrant.clone());
    let collab_vector = CollabVectorService::new(qdrant.clone());
    let recommendations = RecommendationsService::new(
        qdrant.clone(),
        pg.clone(),
        nats.clone(),
        redis_pool.clone(),
        worker.clone(),
        s3_verifier.clone(),
        collab_vector.clone(),
        config.soundwave.clone(),
    );
    let tracks = TracksService::new(crate::modules::tracks::TracksServiceDependencies {
        sc: sc.clone(),
        pg: pg.clone(),
        sync_queue: sync_queue.clone(),
        cold_refresh: cold_refresh.clone(),
        tokens: tokens.clone(),
    });
    let playlists = PlaylistsService::new(PlaylistsDeps {
        sc: sc.clone(),
        pg: pg.clone(),
        sync_queue: sync_queue.clone(),
        cold_refresh: cold_refresh.clone(),
        tokens: tokens.clone(),
        background_jobs: background_jobs.clone(),
    });
    let users = UsersService::new(pg.clone(), cold_refresh.clone());
    let dislikes = DislikesService::new(pg.clone(), events.clone());
    let search = SearchService::new(pg.clone(), cache.clone());
    let history = HistoryService::new(pg.clone());
    let featured = FeaturedService::new(pg.clone());
    let lyrics = LyricsService::new(pg.clone(), background_jobs.clone(), reserve);

    let indexing = IndexingService::new(
        pg.clone(),
        background_jobs.clone(),
        indexing_jobs,
        config.max_track_duration_ms,
    );
    cold_refresh.install_indexing(indexing.clone());

    let likes = LikesService::new(
        pg.clone(),
        sync_queue.clone(),
        indexing.clone(),
        events.clone(),
    );

    let discover = DiscoverService::new(pg.clone(), cache.clone());

    let vibe = crate::modules::search::VibeSearchService::new(
        pg.clone(),
        cache.clone(),
        recommendations.clone(),
        worker.clone(),
        qdrant.clone(),
    );

    events.install_dislikes(dislikes.clone());

    let port = config.port;
    let state = AppState {
        config: config.clone(),
        pg,
        background_jobs,
        http_metrics: std::sync::Arc::new(crate::common::http_metrics::HttpMetrics::new()),
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
        search,
        vibe,
        history,
        featured,
        lyrics,
        collab_vector,
        collab_jobs,
        indexing,
        recommendations,
        discover,
        sync_queue: sync_queue.clone(),
    };

    let app = router::build(state);

    if let Some(tls_cfg) = tls_common::TlsConfig::from_env() {
        info!("starting with TLS (ACME)");
        tls_common::serve(tls_cfg, app).await;
    } else {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        info!(%addr, "starting plain HTTP");
        tls_common::serve_http(addr, tls_common::ProxyProtocolConfig::from_env(), app)
            .await
            .expect("Server error");
    }

    shutdown.cancel();
    info!("backend stopped");
}

async fn build_call_relay(role: &str) -> Option<std::sync::Arc<call_relay::Client>> {
    let endpoint = std::env::var("CALL_CONTROL_ENDPOINT").ok()?;
    if endpoint.is_empty() {
        return None;
    }
    let relay_secret = std::env::var("CALL_RELAY_SECRET").unwrap_or_default();
    if relay_secret.is_empty() {
        tracing::warn!(
            role,
            "CALL_RELAY_SECRET empty; relay will be rejected by server"
        );
    }
    let cfg = call_relay::Config {
        control_endpoint: Some(endpoint),
        upstream_proxy: None,
        instance_id: format!("{role}-{}", std::process::id()),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        relay_secret,
        policy: call_relay::tiers::Policy {
            order: vec![call_relay::Tier::Client],
            timeout_ms: 15_000,
            fallback_on_status_5xx: true,
        },
    };
    match call_relay::Client::connect(cfg).await {
        Ok(c) => {
            tracing::info!(role, "call-relay connected");
            Some(std::sync::Arc::new(c))
        }
        Err(e) => {
            tracing::warn!(role, error = %e, "call-relay connect failed; running without it");
            None
        }
    }
}
