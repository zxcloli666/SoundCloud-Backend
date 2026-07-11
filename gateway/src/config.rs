use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::routes::RouteTable;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HttpMode {
    /// :80 answers 301 → https (default).
    Redirect,
    /// :80 reverse-proxies cleartext to the same upstreams.
    Proxy,
}

pub struct Config {
    pub http_enabled: bool,
    pub https_enabled: bool,
    pub http_port: u16,
    pub https_port: u16,
    pub http_mode: HttpMode,
    pub routes: Arc<RouteTable>,
    pub acme_email: String,
    pub acme_cache_dir: PathBuf,
    pub acme_staging: bool,
    pub shards: usize,
    pub worker_threads: usize,
    pub connect_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    pub shutdown_grace: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let routes = RouteTable::from_env_str(&env_str("GATEWAY_ROUTES", ""))?;
        if routes.is_empty() {
            return Err("GATEWAY_ROUTES is empty — expected lines `host -> http://upstream:port`".into());
        }

        let http_enabled = env_bool("HTTP_ENABLED", true);
        let https_enabled = env_bool("HTTPS_ENABLED", true);
        if !http_enabled && !https_enabled {
            return Err("HTTP_ENABLED and HTTPS_ENABLED are both false — nothing to serve".into());
        }

        let domains = routes.hostnames();
        if https_enabled && domains.is_empty() {
            return Err("HTTPS_ENABLED but every route is a catch-all — no hostname to get an ACME cert for".into());
        }

        let acme_email = env_opt("ACME_EMAIL").unwrap_or_else(|| {
            format!("admin@{}", domains.first().map(String::as_str).unwrap_or("localhost"))
        });

        let par = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);

        Ok(Self {
            http_enabled,
            https_enabled,
            http_port: env_u16("HTTP_PORT", 80),
            https_port: env_u16("HTTPS_PORT", 443),
            http_mode: match env_str("HTTP_MODE", "redirect").to_ascii_lowercase().as_str() {
                "proxy" => HttpMode::Proxy,
                _ => HttpMode::Redirect,
            },
            routes: Arc::new(routes),
            acme_email,
            acme_cache_dir: PathBuf::from(env_str("ACME_CACHE_DIR", "/var/cache/acme")),
            acme_staging: env_bool("ACME_STAGING", false),
            shards: env_usize("GATEWAY_SHARDS", par).max(1),
            worker_threads: env_usize("WORKER_THREADS", par).max(1),
            connect_timeout: Duration::from_millis(env_u64("UPSTREAM_CONNECT_TIMEOUT_MS", 5000)),
            pool_idle_timeout: Duration::from_secs(env_u64("POOL_IDLE_TIMEOUT_SECS", 90)),
            pool_max_idle_per_host: env_usize("POOL_MAX_IDLE_PER_HOST", 4096),
            shutdown_grace: Duration::from_secs(env_u64("SHUTDOWN_GRACE_SECS", 10)),
        })
    }

    pub fn acme_domains(&self) -> Vec<String> {
        self.routes.hostnames()
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn env_str(key: &str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

fn env_bool(key: &str, default: bool) -> bool {
    match env_opt(key) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        None => default,
    }
}

fn env_u16(key: &str, default: u16) -> u16 {
    env_opt(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    env_opt(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env_opt(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}
