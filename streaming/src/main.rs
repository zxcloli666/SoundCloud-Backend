use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, PRAGMA, REFERRER_POLICY};
use axum::http::{HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

mod cleanup;
mod config;
mod db;
mod error;
mod metrics;
mod sc_methods;
mod stream;

#[cfg(test)]
mod env_surface_tests;
#[cfg(test)]
mod secret_surface_tests;
#[cfg(test)]
mod source_tree;

use config::Config;
use db::postgres::PgPool;
use stream::anon::AnonClient;
use stream::cookies_pool::CookiesPool;
use stream::storage::StorageClient;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pg: PgPool,
    pub http_client: wreq::Client,
    pub storage_http: wreq::Client,
    pub anon: Arc<AnonClient>,
    pub cookies: Option<Arc<CookiesPool>>,
    pub storage: Arc<StorageClient>,
    pub decryptor: Option<Arc<decrypt::Engine>>,
}

#[tokio::main]
async fn main() {
    tls_common::init_crypto();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "streaming=info,tower_http=info".parse().unwrap()),
        )
        .init();

    metrics::init();

    let config = Config::from_env();

    if let Some(r) = build_call_relay("streaming").await {
        crate::stream::proxy::install_relay(r);
    }

    let pg = PgPool::connect(&config)
        .await
        .expect("Failed to connect to PostgreSQL");

    let http_client = sc_fingerprint::builder(None)
        .tcp_nodelay(true)
        .pool_max_idle_per_host(16)
        .connect_timeout(std::time::Duration::from_millis(3000))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(wreq::redirect::Policy::limited(10))
        .build()
        .expect("Failed to build HTTP client");

    let storage_http = sc_fingerprint::builder(None)
        .tcp_nodelay(true)
        .pool_max_idle_per_host(16)
        .connect_timeout(std::time::Duration::from_millis(3000))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to build storage HTTP client");

    let storage_passthrough = sc_fingerprint::builder(None)
        .tcp_nodelay(true)
        .pool_max_idle_per_host(16)
        .connect_timeout(std::time::Duration::from_millis(3000))
        .read_timeout(stream::storage::PASSTHROUGH_IDLE_BUDGET)
        .build()
        .expect("Failed to build storage passthrough HTTP client");

    let anon = Arc::new(AnonClient::new(
        http_client.clone(),
        config.sc_proxy_url.clone(),
    ));

    let cookies = if config.cookies_enabled() {
        let pool = Arc::new(CookiesPool::new(
            http_client.clone(),
            &config.sc_proxy_url,
            &config.sc_cookies,
        ));
        pool.log_summary();
        Some(pool)
    } else {
        info!("Cookie-based streaming disabled (no valid SC_COOKIES entries)");
        None
    };

    let storage = Arc::new(StorageClient::new(
        storage_http.clone(),
        storage_passthrough,
        &config,
        pg.clone(),
    ));

    if storage.enabled() {
        if config.storage_public_url != config.storage_url {
            info!(
                "Storage enabled: {} (public: {})",
                config.storage_url, config.storage_public_url
            );
        } else {
            info!("Storage enabled: {}", config.storage_url);
        }
    } else {
        info!("Storage disabled");
    }

    let decryptor = config
        .decrypt_device
        .as_ref()
        .and_then(|p| decrypt::Engine::load(std::path::Path::new(p)).ok())
        .map(Arc::new);
    info!(
        "Decoder engine: {}",
        decryptor
            .as_ref()
            .map(|e| format!("on ({} devices)", e.devices()))
            .unwrap_or_else(|| "off".into())
    );

    let config = Arc::new(config);

    cleanup::task::spawn_cleanup_task((*config).clone(), pg.clone(), storage.clone());
    cleanup::hq_upgrade::spawn_hq_upgrade_task(
        pg.clone(),
        anon.clone(),
        cookies.clone(),
        storage.clone(),
        decryptor.clone(),
        http_client.clone(),
        config.sc_proxy_url.clone(),
    );

    let state = AppState {
        config: config.clone(),
        pg,
        http_client,
        storage_http,
        anon,
        cookies,
        storage,
        decryptor,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers(Any)
        .max_age(std::time::Duration::from_secs(3600));

    let app = Router::new()
        .route("/resolve", get(stream::handler::resolve_track))
        .route("/stream/{track_urn}", get(stream::handler::stream))
        .route("/download/{track_urn}", get(stream::download::download))
        .route(
            "/internal/transcode-upload/{track_urn}",
            post(stream::internal::transcode_upload),
        )
        .route("/internal/wvd", get(stream::internal::serve_wvd))
        .route("/health", get(|| async { "ok" }))
        .route("/metrics", get(serve_metrics))
        .layer(cors)
        .layer(axum::middleware::from_fn(protect_sensitive_responses))
        .layer(axum::middleware::from_fn(track_request))
        .with_state(state);

    if config.premium_only {
        info!("Premium-only mode: non-premium requests are rejected");
    }

    if let Some(tls_cfg) = tls_common::TlsConfig::from_env() {
        info!("Streaming service starting with TLS");
        tls_common::serve(tls_cfg, app).await;
    } else {
        let addr = format!("0.0.0.0:{}", config.port);
        info!("Streaming service starting on {addr}");

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("Failed to bind");

        axum::serve(listener, app)
            .with_graceful_shutdown(tls_common::shutdown_signal())
            .await
            .expect("Server error");
    }
}

async fn serve_metrics(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err((status, message)) =
        stream::internal::check_auth(&headers, &state.config.internal_token)
    {
        return (status, message).into_response();
    }
    let status = state.pg.status();
    metrics::record_pool_state(
        status.max_size,
        status.size,
        status.available,
        status.waiting,
    );
    match metrics::render() {
        Some(body) => Response::builder()
            .header(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4",
            )
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        None => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn track_request(req: Request, next: Next) -> Response {
    let route = metrics::route_label(req.extensions().get::<axum::extract::MatchedPath>());
    let method = metrics::method_label(req.method());
    let started = std::time::Instant::now();
    let response = next.run(req).await;
    metrics::record_request(route, method, response.status().as_u16(), started.elapsed());
    response
}

async fn protect_sensitive_responses(req: Request, next: Next) -> Response {
    let sensitive = is_sensitive_path(req.uri().path());
    let mut response = next.run(req).await;
    if sensitive {
        let headers = response.headers_mut();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("private, no-store, max-age=0"),
        );
        headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    }
    response
}

fn is_sensitive_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    path.starts_with("/stream/") || path.starts_with("/download/") || path == "/metrics"
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
            timeout_ms: 180_000,
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

#[cfg(test)]
mod tests {
    use super::is_sensitive_path;

    #[test]
    fn sensitive_path_policy_includes_malformed_trailing_slashes() {
        assert!(is_sensitive_path("/stream/soundcloud:tracks:42"));
        assert!(is_sensitive_path("/download/soundcloud:tracks:42/"));
        assert!(!is_sensitive_path("/resolve"));
    }
}
