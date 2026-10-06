use std::time::Duration;

use stream_ticket::StreamTicketKey;
use url::Url;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub port: u16,

    pub soundcloud: SoundcloudCfg,
    pub database: DatabaseCfg,
    pub streaming: StreamingCfg,
    pub admin: AdminCfg,
    pub redis: RedisCfg,
    pub admission: AdmissionCfg,
    pub nats: NatsCfg,
    pub qdrant: QdrantCfg,
    pub storage: StorageCfg,
    pub subscriptions: SubscriptionsCfg,
    pub soundwave: SoundwaveCfg,
    pub collab_trigger: CollabTriggerCfg,
    pub cold: ColdCfg,
    pub max_track_duration_ms: i32,
    pub premium_reserve: bool,
    pub reserve_backend: bool,
}

impl AppConfig {
    pub fn is_reserve(&self) -> bool {
        self.premium_reserve || self.reserve_backend
    }
}

#[derive(Clone, Debug)]
pub struct ColdCfg {
    pub track_ttl_sec: u64,
    pub user_ttl_sec: u64,
    pub playlist_ttl_sec: u64,
    pub liked_tracks_ttl_sec: u64,
    pub liked_playlists_ttl_sec: u64,
    pub followings_ttl_sec: u64,
    pub owned_ttl_sec: u64,
    #[allow(dead_code)]
    pub evict_after_sec: u64,
}

#[derive(Clone, Debug)]
pub struct SoundcloudCfg {
    pub proxy_url: String,
    pub proxy_fallback: bool,
}

#[derive(Clone)]
pub struct DatabaseCfg {
    pub url: String,
    pub ssl: DbSslCfg,
    pub pool_max: u32,
    pub acquire_timeout: Duration,
}

impl std::fmt::Debug for DatabaseCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseCfg")
            .field("url", &redact::url(&self.url))
            .field("ssl", &self.ssl)
            .field("pool_max", &self.pool_max)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DbSslCfg {
    pub mode: Option<String>,
    pub root_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StreamingCfg {
    pub service_url: Url,
    pub ticket_key: StreamTicketKey,
}

#[derive(Clone, Debug)]
pub struct AdminCfg {
    pub token: redact::Secret<String>,
}

#[derive(Clone)]
pub struct RedisCfg {
    pub url: String,
}

impl std::fmt::Debug for RedisCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisCfg")
            .field("url", &redact::url(&self.url))
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AdmissionLimitCfg {
    pub per_client: u32,
    pub global: u32,
}

#[derive(Clone, Debug)]
pub struct AdmissionCfg {
    pub window: Duration,
    pub timeout: Duration,
    pub max_in_flight: usize,
    pub login: AdmissionLimitCfg,
    pub link_create: AdmissionLimitCfg,
    pub resolve: AdmissionLimitCfg,
}

impl AdmissionCfg {
    fn from_env() -> Self {
        let login = admission_limit("AUTH_LOGIN", 15, 300);
        let link_create = admission_limit("AUTH_LINK_CREATE", 30, 600);
        let resolve = admission_limit("RESOLVE", 60, 1200);

        Self {
            window: Duration::from_secs(admission_value("ADMISSION_WINDOW_SECONDS", 60)),
            timeout: Duration::from_millis(admission_value("ADMISSION_TIMEOUT_MS", 100)),
            max_in_flight: usize::try_from(admission_value("ADMISSION_MAX_IN_FLIGHT", 16))
                .expect("ADMISSION_MAX_IN_FLIGHT is too large"),
            login,
            link_create,
            resolve,
        }
    }
}

fn admission_limit(prefix: &str, per_client: u32, global: u32) -> AdmissionLimitCfg {
    let limit = AdmissionLimitCfg {
        per_client: admission_u32(&format!("{prefix}_PER_CLIENT"), per_client),
        global: admission_u32(&format!("{prefix}_GLOBAL"), global),
    };
    assert!(
        limit.per_client <= limit.global,
        "{prefix}_PER_CLIENT must not exceed {prefix}_GLOBAL"
    );
    limit
}

#[derive(Clone)]
pub struct NatsCfg {
    pub url: String,
}

impl std::fmt::Debug for NatsCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsCfg")
            .field("url", &redact::url(&self.url))
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct QdrantCfg {
    pub grpc_url: String,
    pub api_key: redact::Secret<String>,
}

#[derive(Clone, Debug)]
pub struct StorageCfg {
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct SubscriptionsCfg {
    pub always_premium: bool,
}

#[derive(Clone, Debug)]
pub struct SoundwaveCfg {
    pub popularity_boost: f64,
    pub artist_cap: usize,
}

#[derive(Clone, Debug)]
pub struct CollabTriggerCfg {
    pub event_threshold: u64,
    pub cooldown: Duration,
}

fn compose_url(user: &str, pass: &str, host: &str, port: u16, name: &str) -> String {
    format!(
        "postgres://{}:{}@{host}:{port}/{}",
        urlencoding::encode(user),
        urlencoding::encode(pass),
        urlencoding::encode(name),
    )
}

fn database_from_env(prefix: &str, default_host: Option<&str>) -> Option<DatabaseCfg> {
    let key = |name: &str| format!("{prefix}{name}");

    let url = match env_opt(&key("DATABASE_URL")) {
        Some(url) => url,
        None => {
            let host =
                env_opt(&key("DATABASE_HOST")).or_else(|| default_host.map(str::to_string))?;
            let port = env_u16(&key("DATABASE_PORT"), 5432);
            let user = env_str(&key("DATABASE_USERNAME"), "soundcloud");
            let pass = env_str(&key("DATABASE_PASSWORD"), "soundcloud");
            let name = env_str(&key("DATABASE_NAME"), "soundcloud_desktop");
            compose_url(&user, &pass, &host, port, &name)
        }
    };

    Some(DatabaseCfg {
        url,
        ssl: DbSslCfg {
            mode: env_opt(&key("DATABASE_SSL_MODE")),
            root_cert: env_opt(&key("DATABASE_SSL_CA")),
            client_cert: env_opt(&key("DATABASE_SSL_CERT")),
            client_key: env_opt(&key("DATABASE_SSL_KEY")),
        },
        pool_max: env_u32(&key("PG_POOL_MAX"), if prefix.is_empty() { 20 } else { 10 }),
        acquire_timeout: Duration::from_secs(env_u64(&key("PG_ACQUIRE_TIMEOUT_SECS"), 10)),
    })
}

fn stream_ticket_key() -> StreamTicketKey {
    let encoded = env_opt("STREAM_TICKET_KEY").expect("STREAM_TICKET_KEY must be set");
    StreamTicketKey::from_base64(&encoded)
        .expect("STREAM_TICKET_KEY must be base64-encoded 32 bytes")
}

fn streaming_service_url() -> Url {
    let mut url = Url::parse(&env_str("STREAMING_SERVICE_URL", "http://localhost:8080"))
        .expect("STREAMING_SERVICE_URL must be an absolute URL");
    assert!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "STREAMING_SERVICE_URL must use HTTP or HTTPS"
    );
    assert!(
        url.query().is_none() && url.fragment().is_none(),
        "STREAMING_SERVICE_URL must not contain a query or fragment"
    );
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    url
}

impl AppConfig {
    pub fn from_env() -> Self {
        let database = database_from_env("", Some("localhost"))
            .expect("core database config: default host is always present");

        Self {
            port: env_u16("PORT", 3000),

            soundcloud: SoundcloudCfg {
                proxy_url: env_str("SC_PROXY_URL", ""),
                proxy_fallback: env_str("SC_PROXY_FALLBACK", "") == "true",
            },

            database,

            streaming: StreamingCfg {
                service_url: streaming_service_url(),
                ticket_key: stream_ticket_key(),
            },

            admin: AdminCfg {
                token: env_str("ADMIN_TOKEN", "").into(),
            },

            redis: RedisCfg {
                url: env_str("REDIS_URL", "redis://localhost:6379"),
            },

            admission: AdmissionCfg::from_env(),

            nats: NatsCfg {
                url: env_str("NATS_URL", "nats://localhost:4222"),
            },

            qdrant: QdrantCfg {
                grpc_url: env_str("QDRANT_URL", "http://localhost:6334"),
                api_key: env_str("QDRANT_API_KEY", "").into(),
            },

            storage: StorageCfg {
                url: env_str("STORAGE_URL", "https://storage.scdinternal.site"),
            },

            subscriptions: SubscriptionsCfg {
                always_premium: env_str("SUBSCRIPTIONS_ALWAYS_PREMIUM", "false") == "true",
            },

            soundwave: SoundwaveCfg {
                popularity_boost: env_f64("SOUNDWAVE_POPULARITY_BOOST", 0.0),
                artist_cap: env_usize("SOUNDWAVE_ARTIST_CAP", 2),
            },

            collab_trigger: CollabTriggerCfg {
                event_threshold: u64::from(env_u32("COLLAB_TRIGGER_EVENTS", 100).max(1)),
                cooldown: Duration::from_millis(env_u64("COLLAB_TRIGGER_COOLDOWN_MS", 600_000)),
            },

            cold: ColdCfg {
                track_ttl_sec: env_u64("COLD_TTL_TRACK_SEC", 21600),
                user_ttl_sec: env_u64("COLD_TTL_USER_SEC", 21600),
                playlist_ttl_sec: env_u64("COLD_TTL_PLAYLIST_SEC", 3600),
                liked_tracks_ttl_sec: env_u64("COLD_TTL_LIKED_TRACKS_SEC", 1800),
                liked_playlists_ttl_sec: env_u64("COLD_TTL_LIKED_PLAYLISTS_SEC", 1800),
                followings_ttl_sec: env_u64("COLD_TTL_FOLLOWINGS_SEC", 1800),
                owned_ttl_sec: env_u64("COLD_TTL_OWNED_SEC", 300),
                evict_after_sec: env_u64("COLD_EVICT_AFTER_SEC", 2_592_000),
            },

            max_track_duration_ms: (env_u64("MAX_TRACK_DURATION_SEC", 420) * 1000) as i32,
            premium_reserve: env_str("PREMIUM_RESERVE", "false") == "true",
            reserve_backend: env_str("RESERVE_BACKEND", "false") == "true",
        }
    }
}

fn env_str(key: &str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn admission_value(key: &str, default: u64) -> u64 {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or_else(|| panic!("{key} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => default,
        Err(std::env::VarError::NotUnicode(_)) => panic!("{key} must be valid UTF-8"),
    }
}

fn admission_u32(key: &str, default: u32) -> u32 {
    u32::try_from(admission_value(key, u64::from(default)))
        .unwrap_or_else(|_| panic!("{key} is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgConnectOptions;
    use std::str::FromStr;

    #[test]
    fn composed_url_round_trips_special_chars() {
        let pass = "p@ss/w?rd#1 &x";
        let url = compose_url("us er", pass, "db.internal", 5433, "sc-ops");
        let opts = PgConnectOptions::from_str(&url).expect("sqlx must parse");
        assert_eq!(opts.get_host(), "db.internal");
        assert_eq!(opts.get_port(), 5433);
        assert_eq!(opts.get_username(), "us er");
        assert_eq!(opts.get_database(), Some("sc-ops"));
    }

    #[test]
    fn debug_does_not_leak_the_password() {
        let cfg = DatabaseCfg {
            url: compose_url("soundcloud", "s3cr3t", "h", 5432, "db"),
            ssl: DbSslCfg::default(),
            pool_max: 1,
            acquire_timeout: Duration::from_secs(1),
        };
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("s3cr3t"), "{rendered}");
        assert!(rendered.contains("soundcloud:***@h:5432"), "{rendered}");
    }

    #[test]
    fn no_configured_secret_can_reach_a_log_through_a_derived_debug() {
        let admin = AdminCfg {
            token: "s3cr3t-admin-token".to_owned().into(),
        };
        let qdrant = QdrantCfg {
            grpc_url: "http://vectors:6334".to_owned(),
            api_key: "s3cr3t-qdrant-key".to_owned().into(),
        };
        let redis = RedisCfg {
            url: "redis://user:s3cr3t-redis-pw@cache:6379".to_owned(),
        };
        let nats = NatsCfg {
            url: "nats://user:s3cr3t-nats-pw@bus:4222".to_owned(),
        };

        for rendered in [
            format!("{admin:?}"),
            format!("{qdrant:?}"),
            format!("{redis:?}"),
            format!("{nats:?}"),
        ] {
            assert!(
                !rendered.contains("s3cr3t"),
                "one `tracing::debug!(?config)` anywhere would write this to disk: {rendered}"
            );
        }

        assert!(
            format!("{qdrant:?}").contains("vectors:6334"),
            "masking must not swallow the part that makes the line worth logging"
        );
    }
}
