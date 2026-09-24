pub(crate) mod database;
mod env;
mod oauth;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use backend_contracts::COLLAB_MAX_MIN_COUNT;
use backend_contracts::worker_contract::WorkerLane;

pub use database::{DatabaseConfig, PoolConfig, SessionLimits};
pub use env::ConfigError;
pub use oauth::{OAuthAppBootstrap, OAuthConfig};

const DURATION_MAX_BATCH_SIZE: i64 = 500;
const DURATION_MAX_CONCURRENCY: usize = 32;
const DURATION_MIN_REQUEST_GAP: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct JobsConfig {
    pub instance_id: String,
    pub health_bind: SocketAddr,
    pub nats: NatsConfig,
    pub qdrant: QdrantConfig,
    pub collab: CollabConfig,
    pub taste: TasteConfig,
    pub durations: DurationConfig,
    pub indexing: IndexingConfig,
    pub sync_queue: SyncQueueConfig,
    pub account_walk: AccountWalkConfig,
    pub crawl: CrawlConfig,
    pub wanted: WantedConfig,
    pub enrich: EnrichConfig,
    pub lyrics: LyricsConfig,
    pub worker_dispatch: WorkerDispatchConfig,
    pub admin_maintenance: AdminMaintenanceConfig,
    pub playlist_reconcile: PlaylistReconcileConfig,
    pub main_database: DatabaseConfig,
    pub ops_database: DatabaseConfig,
    pub queue: QueueConfig,
    pub subscriptions: SubscriptionsConfig,
    pub subscriptions_always_premium: bool,
    pub schedules: JobScheduleConfig,
    pub oauth: OAuthConfig,
    pub shutdown_grace: Duration,
}

#[derive(Clone, Debug)]
pub struct AccountWalkConfig {
    pub walk_interval_days: i64,
    pub lease_seconds: i64,
    pub batch: i64,
    pub concurrency: usize,
}

impl AccountWalkConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            walk_interval_days: env::positive_u64("DISCOVERY_ACCOUNT_WALK_DAYS", 7)? as i64,
            lease_seconds: env::positive_u64("DISCOVERY_ACCOUNT_WALK_LEASE_SECONDS", 1_800)? as i64,
            batch: env::positive_u64("DISCOVERY_ACCOUNT_WALK_BATCH", 64)? as i64,
            concurrency: env::non_zero_usize("DISCOVERY_ACCOUNT_WALK_CONCURRENCY", 16)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CrawlConfig {
    pub genius_batch: i64,
    pub genius_concurrency: usize,
    pub mb_batch: i64,
    pub mb_concurrency: usize,
    pub identity_batch: i64,
    pub lease_seconds: i64,
    pub recrawl_days: i64,
    pub max_fails: i16,
    pub post_crawl_wanted_max: i64,
}

impl CrawlConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let max_fails = env::positive_u64("DISCOVERY_MAX_FAILS", 8)?;
        let max_fails = i16::try_from(max_fails).map_err(|_| ConfigError::Invalid {
            key: "DISCOVERY_MAX_FAILS".to_owned(),
            reason: "must fit a signed 16-bit integer".to_owned(),
        })?;
        Ok(Self {
            genius_batch: env::positive_u64("DISCOVERY_BATCH", 256)? as i64,
            genius_concurrency: env::non_zero_usize("DISCOVERY_GENIUS_CONCURRENCY", 64)?,
            mb_batch: env::positive_u64("DISCOVERY_MB_BATCH", 32)? as i64,
            mb_concurrency: env::non_zero_usize("DISCOVERY_MB_CONCURRENCY", 8)?,
            identity_batch: env::positive_u64("DISCOVERY_IDENTITY_BATCH", 128)? as i64,
            lease_seconds: env::positive_u64("DISCOVERY_LEASE_SECONDS", 1_800)? as i64,
            recrawl_days: env::positive_u64("DISCOVERY_RECRAWL_DAYS", 14)? as i64,
            max_fails,
            post_crawl_wanted_max: env::positive_u64("DISCOVERY_POST_CRAWL_WANTED_MAX", 256)?
                as i64,
        })
    }
}

#[derive(Clone, Debug)]
pub struct WantedConfig {
    pub batch: i64,
    pub search_concurrency: usize,
    pub lease_seconds: i64,
    pub max_attempts: i16,
    pub ai_enabled: bool,
    pub ai_timeout_ms: u64,
    pub ai_daily_budget: u64,
}

impl WantedConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let max_attempts = env::positive_u64("WANTED_RESOLVE_MAX_ATTEMPTS", 8)?;
        let max_attempts = i16::try_from(max_attempts).map_err(|_| ConfigError::Invalid {
            key: "WANTED_RESOLVE_MAX_ATTEMPTS".to_owned(),
            reason: "must fit a signed 16-bit integer".to_owned(),
        })?;
        Ok(Self {
            batch: env::positive_u64("WANTED_RESOLVE_BATCH", 128)? as i64,
            search_concurrency: env::non_zero_usize("WANTED_RESOLVE_CONCURRENCY", 64)?,
            lease_seconds: env::positive_u64("WANTED_RESOLVE_LEASE_SECONDS", 1_800)? as i64,
            max_attempts,
            ai_enabled: env::boolean("ENRICH_AI_ENABLED", true)?,
            ai_timeout_ms: env::positive_u64("ENRICH_AI_TIMEOUT_MS", 20_000)?,
            ai_daily_budget: env::positive_u64("ENRICH_AI_DAILY_BUDGET", 5_000)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AdminMaintenanceConfig {
    pub scan_batch: i64,
}

impl AdminMaintenanceConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            scan_batch: env::positive_u64("ADMIN_MAINTENANCE_SCAN_BATCH", 2_000)? as i64,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlaylistReconcileConfig {
    pub sweep_batch: i64,
    pub sweep_owner_share: i64,
    pub claim_seconds: i64,
    pub legacy_drain_batch: i64,
    pub membership_remote_apply: bool,
}

impl PlaylistReconcileConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            sweep_batch: env::positive_u64("PLAYLIST_RECONCILE_SWEEP_BATCH", 128)? as i64,
            sweep_owner_share: env::positive_u64("PLAYLIST_RECONCILE_OWNER_SHARE", 8)? as i64,
            claim_seconds: env::positive_u64("PLAYLIST_RECONCILE_CLAIM_SECONDS", 300)? as i64,
            legacy_drain_batch: env::positive_u64("PLAYLIST_LEGACY_DRAIN_BATCH", 500)? as i64,
            membership_remote_apply: env::boolean("PLAYLIST_MEMBERSHIP_REMOTE_APPLY", false)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WorkerDispatchConfig {
    pub embed_lyrics: bool,
    pub index_audio: bool,
    pub transcribe: bool,
    pub lyrics_align_rejected_retry_days: u64,
}

impl WorkerDispatchConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            embed_lyrics: env::boolean("EMBED_LYRICS_DISPATCH", false)?,
            index_audio: env::boolean("INDEX_AUDIO_DISPATCH", false)?,
            transcribe: env::boolean("TRANSCRIBE_DISPATCH", false)?,
            lyrics_align_rejected_retry_days: env::positive_u64(
                "LYRICS_ALIGN_REJECTED_RETRY_DAYS",
                30,
            )?,
        })
    }

    pub fn required_worker_lanes(&self) -> Vec<WorkerLane> {
        let switched = [
            (self.index_audio, WorkerLane::Audio),
            (self.embed_lyrics, WorkerLane::Lyrics),
            (self.transcribe, WorkerLane::Transcribe),
        ]
        .into_iter()
        .filter_map(|(enabled, lane)| enabled.then_some(lane));
        [WorkerLane::Encode, WorkerLane::Ai, WorkerLane::Collab]
            .into_iter()
            .chain(switched)
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct LyricsConfig {
    pub batch: i64,
    pub concurrency: usize,
    pub claim_seconds: i64,
    pub backfill_batch: i64,
    pub musixmatch_base: String,
}

impl LyricsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            batch: env::positive_u64("LYRICS_LOOKUP_BATCH", 512)? as i64,
            concurrency: env::non_zero_usize("LYRICS_LOOKUP_CONCURRENCY", 128)?,
            claim_seconds: env::positive_u64("LYRICS_LOOKUP_CLAIM_SECONDS", 1_200)? as i64,
            backfill_batch: env::positive_u64("LYRICS_LOOKUP_BACKFILL_BATCH", 20_000)? as i64,
            musixmatch_base: env::optional("MUSIXMATCH_API_BASE")
                .unwrap_or_else(|| "https://apic-desktop.musixmatch.com/ws/1.1".to_owned()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct EnrichConfig {
    pub concurrency: usize,
    pub max_attempts: i16,
    pub proxy_url: String,
    pub musicbrainz_rate_limit_ms: u64,
    pub genius_access_token: redact::Secret<String>,
    pub genius_max_concurrent_scrapes: usize,
    pub ai_enabled: bool,
    pub ai_timeout_ms: u64,
    pub ai_daily_budget: u64,
}

impl EnrichConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let max_attempts = env::positive_u64("ENRICH_MAX_ATTEMPTS", 5)?;
        let max_attempts = i16::try_from(max_attempts).map_err(|_| ConfigError::Invalid {
            key: "ENRICH_MAX_ATTEMPTS".to_owned(),
            reason: "must fit a signed 16-bit integer".to_owned(),
        })?;
        Ok(Self {
            concurrency: env::non_zero_usize("ENRICH_CONSUMER_CONCURRENCY", 32)?,
            max_attempts,
            proxy_url: env::optional("SC_PROXY_URL").unwrap_or_default(),
            musicbrainz_rate_limit_ms: env::positive_u64("ENRICH_MB_RATE_LIMIT_MS", 1100)?,
            genius_access_token: env::optional("GENIUS_ACCESS_TOKEN")
                .unwrap_or_default()
                .into(),
            genius_max_concurrent_scrapes: env::non_zero_usize(
                "GENIUS_MAX_CONCURRENT_SCRAPES",
                128,
            )?,
            ai_enabled: env::boolean("ENRICH_AI_ENABLED", true)?,
            ai_timeout_ms: env::positive_u64("ENRICH_AI_TIMEOUT_MS", 20_000)?,
            ai_daily_budget: env::positive_u64("ENRICH_AI_DAILY_BUDGET", 5_000)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct QueueConfig {
    pub core_fast: QueueLaneConfig,
    pub core_bulk: QueueLaneConfig,
    pub ops: QueueLaneConfig,
    pub poll_interval: Duration,
    pub lease_duration: Duration,
    pub heartbeat_interval: Duration,
    pub job_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct QueueLaneConfig {
    pub concurrency: usize,
    pub claim_batch: usize,
}

#[derive(Clone, Debug)]
pub struct NatsConfig {
    pub url: String,
    pub job_ingress_concurrency: usize,
    pub impression_concurrency: usize,
    pub retry_delay: Duration,
    pub max_age: Duration,
    pub job_stream_max_bytes: i64,
    pub impression_stream_max_bytes: i64,
}

#[derive(Clone, Debug)]
pub struct QdrantConfig {
    pub grpc_url: String,
    pub api_key: redact::Secret<String>,
}

#[derive(Clone, Debug)]
pub struct IndexingConfig {
    pub streaming_url: url::Url,
    pub internal_token: redact::Secret<String>,
}

#[derive(Clone, Debug)]
pub struct CollabConfig {
    pub min_count: u32,
    pub min_sessions: usize,
    pub max_object_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct TasteConfig {
    pub dispatch: bool,
    pub export_interval: Duration,
    pub refresh_interval: Duration,
    pub history_days: u32,
    pub min_users: usize,
    pub epochs: u32,
    pub batch_size: u32,
    pub negatives: u32,
    pub seed: u32,
    pub keep_versions: usize,
    pub max_object_bytes: Option<usize>,
}

impl TasteConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            dispatch: env::boolean("TASTE_DISPATCH", false)?,
            export_interval: Duration::from_secs(env::positive_u64(
                "TASTE_EXPORT_INTERVAL_S",
                86_400,
            )?),
            refresh_interval: Duration::from_secs(env::positive_u64(
                "TASTE_REFRESH_INTERVAL_S",
                300,
            )?),
            history_days: positive(
                "TASTE_HISTORY_DAYS",
                env::parse("TASTE_HISTORY_DAYS", "180")?,
            )?,
            min_users: env::non_zero_usize("TASTE_MIN_USERS", 500)?,
            epochs: positive("TASTE_EPOCHS", env::parse("TASTE_EPOCHS", "10")?)?,
            batch_size: positive("TASTE_BATCH_SIZE", env::parse("TASTE_BATCH_SIZE", "512")?)?,
            negatives: positive("TASTE_NEGATIVES", env::parse("TASTE_NEGATIVES", "1024")?)?,
            seed: positive("TASTE_SEED", env::parse("TASTE_SEED", "1")?)?,
            keep_versions: env::non_zero_usize("TASTE_KEEP_VERSIONS", 3)?,
            max_object_bytes: env::optional("TASTE_MAX_OBJECT_BYTES")
                .map(|_| env::non_zero_usize("TASTE_MAX_OBJECT_BYTES", 1))
                .transpose()?,
        })
    }
}

fn positive(key: &str, value: u32) -> Result<u32, ConfigError> {
    if value == 0 {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    Ok(value)
}

#[derive(Clone, Debug)]
pub struct DurationConfig {
    pub api_v2_url: url::Url,
    pub web_url: url::Url,
    pub proxy_url: Option<url::Url>,
    pub proxy_fallback: bool,
    pub batch_size: i64,
    pub concurrency: usize,
    pub request_gap: Duration,
    pub max_track_duration_ms: i32,
}

#[derive(Clone, Debug)]
pub struct SyncQueueConfig {
    pub api_url: url::Url,
    pub proxy_url: Option<url::Url>,
    pub storage_url: url::Url,
    pub storage_token: redact::Secret<String>,
    pub concurrency: usize,
    pub claim_batch: usize,
    pub lease_duration: Duration,
}

#[derive(Clone, Debug)]
pub struct SubscriptionsConfig {
    pub snapshot_dir: PathBuf,
    pub max_file_bytes: usize,
    pub max_entries: usize,
}

#[derive(Clone, Debug)]
pub struct JobScheduleConfig {
    pub enrich_enabled: bool,
    pub collab_train_enabled: bool,
    pub collab_train_seconds: i32,
    pub duration_resolver_seconds: i32,
    pub recommendation_colike_seconds: i32,
    pub recommendation_quality_backfill_seconds: i32,
    pub recommendation_quality_train_seconds: i32,
    pub recommendation_wave_priority_seconds: i32,
    pub recommendation_wave_priority_shards: i64,
    pub discover_interest_seconds: i32,
    pub discover_interest_shards: i64,
    pub discover_artist_plays_shards: i64,
    pub discover_interest_enabled: bool,
    pub catalog_crawl_enabled: bool,
    pub catalog_crawl_seconds: i32,
    pub wanted_resolve_seconds: i32,
    pub lyrics_lookup_seconds: i32,
    pub playlist_reconcile_sweep_seconds: i32,
}

impl JobsConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let instance_id =
            match env::optional("JOBS_INSTANCE_ID").or_else(|| env::optional("HOSTNAME")) {
                Some(instance_id) => instance_id,
                None => format!("jobs-{}", std::process::id()),
            };

        let queue = QueueConfig {
            core_fast: QueueLaneConfig::from_env("JOBS_CORE_FAST", 4)?,
            core_bulk: QueueLaneConfig::from_env("JOBS_CORE_BULK", 2)?,
            ops: QueueLaneConfig::from_env("JOBS_OPS", 4)?,
            poll_interval: Duration::from_millis(env::positive_u64("JOBS_POLL_INTERVAL_MS", 250)?),
            lease_duration: Duration::from_secs(env::positive_u64("JOBS_LEASE_SECONDS", 300)?),
            heartbeat_interval: Duration::from_secs(env::positive_u64(
                "JOBS_HEARTBEAT_SECONDS",
                30,
            )?),
            job_timeout: Duration::from_secs(env::positive_u64("JOBS_JOB_TIMEOUT_SECONDS", 900)?),
        };
        queue.validate()?;

        let config = Self {
            instance_id,
            health_bind: env::parse("JOBS_HEALTH_BIND", "0.0.0.0:3001")?,
            nats: NatsConfig::from_env()?,
            qdrant: QdrantConfig::from_env()?,
            collab: CollabConfig::from_env()?,
            taste: TasteConfig::from_env()?,
            durations: DurationConfig::from_env()?,
            indexing: IndexingConfig::from_env()?,
            sync_queue: SyncQueueConfig::from_env()?,
            account_walk: AccountWalkConfig::from_env()?,
            crawl: CrawlConfig::from_env()?,
            wanted: WantedConfig::from_env()?,
            enrich: EnrichConfig::from_env()?,
            lyrics: LyricsConfig::from_env()?,
            worker_dispatch: WorkerDispatchConfig::from_env()?,
            admin_maintenance: AdminMaintenanceConfig::from_env()?,
            playlist_reconcile: PlaylistReconcileConfig::from_env()?,
            main_database: DatabaseConfig::from_env("", true)?,
            ops_database: DatabaseConfig::from_env("OPS_", false)?,
            queue,
            subscriptions: SubscriptionsConfig::from_env()?,
            subscriptions_always_premium: env::boolean("SUBSCRIPTIONS_ALWAYS_PREMIUM", false)?,
            schedules: JobScheduleConfig::from_env()?,
            oauth: OAuthConfig::from_env()?,
            shutdown_grace: Duration::from_secs(env::positive_u64(
                "JOBS_SHUTDOWN_GRACE_SECONDS",
                30,
            )?),
        };
        config.validate_work_windows()?;
        Ok(config)
    }

    fn validate_work_windows(&self) -> Result<(), ConfigError> {
        let timeout_seconds =
            i64::try_from(self.queue.job_timeout.as_secs()).map_err(|_| ConfigError::Invalid {
                key: "JOBS_JOB_TIMEOUT_SECONDS".to_owned(),
                reason: "must fit a signed 64-bit integer".to_owned(),
            })?;
        for (key, lease_seconds) in [
            (
                "DISCOVERY_ACCOUNT_WALK_LEASE_SECONDS",
                self.account_walk.lease_seconds,
            ),
            ("DISCOVERY_LEASE_SECONDS", self.crawl.lease_seconds),
            ("WANTED_RESOLVE_LEASE_SECONDS", self.wanted.lease_seconds),
            ("LYRICS_LOOKUP_CLAIM_SECONDS", self.lyrics.claim_seconds),
        ] {
            if lease_seconds < timeout_seconds {
                return Err(ConfigError::Invalid {
                    key: key.to_owned(),
                    reason: "must cover JOBS_JOB_TIMEOUT_SECONDS".to_owned(),
                });
            }
        }
        validate_window(
            "DISCOVERY_ACCOUNT_WALK_BATCH",
            self.account_walk.batch,
            self.account_walk.concurrency,
            4,
        )?;
        validate_window(
            "DISCOVERY_BATCH",
            self.crawl.genius_batch,
            self.crawl.genius_concurrency,
            4,
        )?;
        validate_window(
            "DISCOVERY_MB_BATCH",
            self.crawl.mb_batch,
            self.crawl.mb_concurrency,
            4,
        )?;
        validate_window(
            "DISCOVERY_IDENTITY_BATCH",
            self.crawl.identity_batch,
            self.crawl.genius_concurrency,
            4,
        )?;
        validate_window(
            "WANTED_RESOLVE_BATCH",
            self.wanted.batch,
            self.wanted.search_concurrency,
            2,
        )?;
        validate_window(
            "LYRICS_LOOKUP_BATCH",
            self.lyrics.batch,
            self.lyrics.concurrency,
            4,
        )
    }
}

fn validate_window(
    key: &str,
    batch: i64,
    concurrency: usize,
    maximum_waves: i64,
) -> Result<(), ConfigError> {
    let concurrency = i64::try_from(concurrency).map_err(|_| ConfigError::Invalid {
        key: key.to_owned(),
        reason: "concurrency must fit a signed 64-bit integer".to_owned(),
    })?;
    let maximum = concurrency.saturating_mul(maximum_waves);
    if batch > maximum {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: format!("must not exceed {maximum} for the configured concurrency"),
        });
    }
    Ok(())
}

impl DurationConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let batch_size = env::positive_u64("DURATION_RESOLVER_BATCH_SIZE", 500)?;
        let batch_size = i64::try_from(batch_size).map_err(|_| ConfigError::Invalid {
            key: "DURATION_RESOLVER_BATCH_SIZE".to_owned(),
            reason: "must fit a signed 64-bit integer".to_owned(),
        })?;
        let max_seconds = env::positive_u64("MAX_TRACK_DURATION_SEC", 420)?;
        let max_track_duration_ms = max_seconds
            .checked_mul(1_000)
            .and_then(|milliseconds| i32::try_from(milliseconds).ok())
            .ok_or_else(|| ConfigError::Invalid {
                key: "MAX_TRACK_DURATION_SEC".to_owned(),
                reason: "must fit PostgreSQL integer milliseconds".to_owned(),
            })?;

        let concurrency = env::non_zero_usize("DURATION_RESOLVER_CONCURRENCY", 4)?;
        let request_gap_ms = env::positive_u64("DURATION_RESOLVER_REQUEST_GAP_MS", 150)?;
        let config = Self {
            api_v2_url: service_url(
                "SOUNDCLOUD_API_V2_URL",
                env::optional("SOUNDCLOUD_API_V2_URL")
                    .unwrap_or_else(|| "https://api-v2.soundcloud.com".to_owned()),
            )?,
            web_url: service_url(
                "SOUNDCLOUD_WEB_URL",
                env::optional("SOUNDCLOUD_WEB_URL")
                    .unwrap_or_else(|| "https://soundcloud.com".to_owned()),
            )?,
            proxy_url: env::optional("SC_PROXY_URL")
                .filter(|value| !value.trim().is_empty())
                .map(|value| http_url("SC_PROXY_URL", value))
                .transpose()?,
            proxy_fallback: env::boolean("SC_PROXY_FALLBACK", false)?,
            batch_size,
            concurrency,
            request_gap: Duration::from_millis(request_gap_ms),
            max_track_duration_ms,
        };
        config.validate_limits()?;
        Ok(config)
    }

    fn validate_limits(&self) -> Result<(), ConfigError> {
        if self.batch_size > DURATION_MAX_BATCH_SIZE {
            return Err(ConfigError::Invalid {
                key: "DURATION_RESOLVER_BATCH_SIZE".to_owned(),
                reason: format!("must not exceed {DURATION_MAX_BATCH_SIZE}"),
            });
        }
        if self.concurrency > DURATION_MAX_CONCURRENCY {
            return Err(ConfigError::Invalid {
                key: "DURATION_RESOLVER_CONCURRENCY".to_owned(),
                reason: format!("must not exceed {DURATION_MAX_CONCURRENCY}"),
            });
        }
        if self.request_gap < DURATION_MIN_REQUEST_GAP {
            return Err(ConfigError::Invalid {
                key: "DURATION_RESOLVER_REQUEST_GAP_MS".to_owned(),
                reason: format!("must be at least {}", DURATION_MIN_REQUEST_GAP.as_millis()),
            });
        }
        Ok(())
    }
}

impl CollabConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let min_count = env::parse::<u32>("COLLAB_MIN_COUNT", "3")?;
        if min_count == 0 {
            return Err(ConfigError::Invalid {
                key: "COLLAB_MIN_COUNT".to_owned(),
                reason: "must be greater than zero".to_owned(),
            });
        }
        if min_count > COLLAB_MAX_MIN_COUNT {
            return Err(ConfigError::Invalid {
                key: "COLLAB_MIN_COUNT".to_owned(),
                reason: format!("must not exceed {COLLAB_MAX_MIN_COUNT}"),
            });
        }
        Ok(Self {
            min_count,
            min_sessions: env::non_zero_usize("COLLAB_MIN_SESSIONS", 20)?,
            max_object_bytes: env::non_zero_usize("COLLAB_OBJECT_MAX_BYTES", 512 * 1024 * 1024)?,
        })
    }
}

impl IndexingConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let mut streaming_url = http_url(
            "STREAMING_SERVICE_URL",
            env::required("STREAMING_SERVICE_URL")?,
        )?;
        if streaming_url.query().is_some() || streaming_url.fragment().is_some() {
            return Err(ConfigError::Invalid {
                key: "STREAMING_SERVICE_URL".to_owned(),
                reason: "must not contain a query or fragment".to_owned(),
            });
        }
        let path = streaming_url.path().trim_end_matches('/').to_owned();
        streaming_url.set_path(&path);

        Ok(Self {
            streaming_url,
            internal_token: env::required("INTERNAL_TOKEN")?.into(),
        })
    }
}

impl SyncQueueConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let api_url = https_url(
            "SOUNDCLOUD_API_URL",
            env::optional("SOUNDCLOUD_API_URL")
                .unwrap_or_else(|| "https://api.soundcloud.com".to_owned()),
        )?;
        let proxy_url = env::optional("SC_PROXY_URL")
            .filter(|value| !value.trim().is_empty())
            .map(|value| http_url("SC_PROXY_URL", value))
            .transpose()?;
        let storage_url = http_url("STORAGE_URL", env::required("STORAGE_URL")?)?;
        let storage_token = env::required("STORAGE_TOKEN")?.into();
        let concurrency = env::non_zero_usize("SYNC_QUEUE_CONCURRENCY", 16)?;
        let claim_batch = env::non_zero_usize("SYNC_QUEUE_CLAIM_BATCH", concurrency)?;
        let lease_duration =
            Duration::from_secs(env::positive_u64("SYNC_QUEUE_LEASE_SECONDS", 300)?);
        let config = Self {
            api_url,
            proxy_url,
            storage_url,
            storage_token,
            concurrency,
            claim_batch,
            lease_duration,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.claim_batch > self.concurrency {
            return Err(ConfigError::Invalid {
                key: "SYNC_QUEUE_CLAIM_BATCH".to_owned(),
                reason: "must not exceed SYNC_QUEUE_CONCURRENCY".to_owned(),
            });
        }
        if self.lease_duration < Duration::from_secs(300) {
            return Err(ConfigError::Invalid {
                key: "SYNC_QUEUE_LEASE_SECONDS".to_owned(),
                reason: "must be at least 300 seconds".to_owned(),
            });
        }
        Ok(())
    }
}

fn https_url(key: &str, value: String) -> Result<url::Url, ConfigError> {
    let url = value
        .parse::<url::Url>()
        .map_err(|error| ConfigError::Invalid {
            key: key.to_owned(),
            reason: error.to_string(),
        })?;
    if url.scheme() != "https" {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must use https".to_owned(),
        });
    }
    Ok(url)
}

fn http_url(key: &str, value: String) -> Result<url::Url, ConfigError> {
    let url = value
        .parse::<url::Url>()
        .map_err(|error| ConfigError::Invalid {
            key: key.to_owned(),
            reason: error.to_string(),
        })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must use http or https".to_owned(),
        });
    }
    Ok(url)
}

fn service_url(key: &str, value: String) -> Result<url::Url, ConfigError> {
    let mut url = http_url(key, value)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must not contain a query or fragment".to_owned(),
        });
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}

impl QdrantConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let grpc_url =
            env::optional("QDRANT_URL").unwrap_or_else(|| "http://localhost:6334".to_owned());
        let url = grpc_url
            .parse::<url::Url>()
            .map_err(|error| ConfigError::Invalid {
                key: "QDRANT_URL".to_owned(),
                reason: error.to_string(),
            })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ConfigError::Invalid {
                key: "QDRANT_URL".to_owned(),
                reason: "must use http or https".to_owned(),
            });
        }
        Ok(Self {
            grpc_url,
            api_key: env::optional("QDRANT_API_KEY").unwrap_or_default().into(),
        })
    }
}

impl JobScheduleConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            enrich_enabled: env::boolean("ENRICH_ENABLED", true)?,
            collab_train_enabled: env::boolean("COLLAB_AUTO_TRAIN", true)?,
            collab_train_seconds: schedule_interval("COLLAB_TRAIN_INTERVAL_SECONDS", 6 * 60 * 60)?,
            duration_resolver_seconds: schedule_interval("DURATION_RESOLVER_INTERVAL_SECONDS", 60)?,
            recommendation_colike_seconds: schedule_interval(
                "RECS_COLIKE_REBUILD_SECS",
                6 * 60 * 60,
            )?,
            recommendation_quality_backfill_seconds: schedule_interval(
                "RECS_QUALITY_BACKFILL_SECS",
                10 * 60,
            )?,
            recommendation_quality_train_seconds: schedule_interval(
                "RECS_TRAINER_CRON_SECS",
                6 * 60 * 60,
            )?,
            recommendation_wave_priority_seconds: schedule_interval(
                "RECS_WAVE_BUMP_SECS",
                60 * 60,
            )?,
            recommendation_wave_priority_shards: i64::try_from(env::positive_u64(
                "RECS_WAVE_BUMP_SHARDS",
                16,
            )?)
            .map_err(|_| ConfigError::Invalid {
                key: "RECS_WAVE_BUMP_SHARDS".to_owned(),
                reason: "must fit a signed 64-bit integer".to_owned(),
            })?,
            discover_interest_seconds: schedule_interval(
                "DISCOVERY_INTEREST_INTERVAL_SEC",
                60 * 60,
            )?,
            discover_interest_shards: i64::try_from(env::positive_u64(
                "DISCOVERY_INTEREST_SHARDS",
                8,
            )?)
            .map_err(|_| ConfigError::Invalid {
                key: "DISCOVERY_INTEREST_SHARDS".to_owned(),
                reason: "must fit a signed 64-bit integer".to_owned(),
            })?,
            discover_artist_plays_shards: i64::try_from(env::positive_u64(
                "DISCOVERY_ARTIST_PLAYS_SHARDS",
                8,
            )?)
            .map_err(|_| ConfigError::Invalid {
                key: "DISCOVERY_ARTIST_PLAYS_SHARDS".to_owned(),
                reason: "must fit a signed 64-bit integer".to_owned(),
            })?,
            discover_interest_enabled: env::boolean("DISCOVERY_ENABLED", true)?,
            catalog_crawl_enabled: env::boolean("DISCOVERY_ENABLED", true)?,
            catalog_crawl_seconds: schedule_interval("DISCOVERY_CRAWL_INTERVAL_SECONDS", 60)?,
            wanted_resolve_seconds: schedule_interval("WANTED_RESOLVE_INTERVAL_SECONDS", 60)?,
            lyrics_lookup_seconds: schedule_interval("LYRICS_LOOKUP_INTERVAL_SECONDS", 60)?,
            playlist_reconcile_sweep_seconds: schedule_interval(
                "PLAYLIST_RECONCILE_SWEEP_SECONDS",
                60,
            )?,
        })
    }
}

fn schedule_interval(key: &str, default: u64) -> Result<i32, ConfigError> {
    let seconds = env::positive_u64(key, default)?;
    if seconds < 60 {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must be at least 60 seconds".to_owned(),
        });
    }
    i32::try_from(seconds).map_err(|_| ConfigError::Invalid {
        key: key.to_owned(),
        reason: "must fit PostgreSQL integer seconds".to_owned(),
    })
}

impl NatsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            url: env::optional("NATS_URL").unwrap_or_else(|| "nats://127.0.0.1:4222".to_owned()),
            job_ingress_concurrency: env::non_zero_usize("JOBS_NATS_JOB_INGRESS_CONCURRENCY", 2)?,
            impression_concurrency: env::non_zero_usize("JOBS_NATS_IMPRESSION_CONCURRENCY", 2)?,
            retry_delay: Duration::from_secs(env::positive_u64("JOBS_NATS_RETRY_SECONDS", 30)?),
            max_age: Duration::from_secs(env::positive_u64(
                "JOBS_NATS_MAX_AGE_SECONDS",
                3 * 24 * 60 * 60,
            )?),
            job_stream_max_bytes: stream_bytes(
                "JOBS_NATS_JOB_STREAM_MAX_BYTES",
                1024 * 1024 * 1024,
            )?,
            impression_stream_max_bytes: stream_bytes(
                "JOBS_NATS_IMPRESSION_STREAM_MAX_BYTES",
                8 * 1024 * 1024 * 1024,
            )?,
        })
    }
}

fn stream_bytes(key: &str, default: u64) -> Result<i64, ConfigError> {
    let value = env::positive_u64(key, default)?;
    i64::try_from(value).map_err(|_| ConfigError::Invalid {
        key: key.to_owned(),
        reason: "must fit a signed 64-bit byte count".to_owned(),
    })
}

impl SubscriptionsConfig {
    fn from_env() -> Result<Self, ConfigError> {
        let snapshot_dir = match env::optional("SUBSCRIPTIONS_SNAPSHOT_DIR") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("/snapshots"),
        };
        let config = Self {
            snapshot_dir,
            max_file_bytes: env::non_zero_usize(
                "SUBSCRIPTIONS_SNAPSHOT_MAX_BYTES",
                16 * 1024 * 1024,
            )?,
            max_entries: env::non_zero_usize("SUBSCRIPTIONS_SNAPSHOT_MAX_ENTRIES", 100_000)?,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.max_file_bytes < 1_024 {
            return Err(ConfigError::Invalid {
                key: "SUBSCRIPTIONS_SNAPSHOT_MAX_BYTES".to_owned(),
                reason: "must be at least 1024".to_owned(),
            });
        }
        if self.max_file_bytes.checked_add(1).is_none()
            || u64::try_from(self.max_file_bytes).is_err()
        {
            return Err(ConfigError::Invalid {
                key: "SUBSCRIPTIONS_SNAPSHOT_MAX_BYTES".to_owned(),
                reason: "is too large for bounded file reads".to_owned(),
            });
        }

        let query_limit = self
            .max_entries
            .checked_add(1)
            .and_then(|value| i64::try_from(value).ok());
        if query_limit.is_none() {
            return Err(ConfigError::Invalid {
                key: "SUBSCRIPTIONS_SNAPSHOT_MAX_ENTRIES".to_owned(),
                reason: "is too large for PostgreSQL LIMIT".to_owned(),
            });
        }

        Ok(())
    }
}

impl QueueConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        self.core_fast.validate("JOBS_CORE_FAST")?;
        self.core_bulk.validate("JOBS_CORE_BULK")?;
        self.ops.validate("JOBS_OPS")?;

        let lease_renews_safely = self
            .heartbeat_interval
            .checked_mul(3)
            .is_some_and(|minimum_lease| minimum_lease <= self.lease_duration);
        if !lease_renews_safely {
            return Err(ConfigError::Invalid {
                key: "JOBS_LEASE_SECONDS".to_owned(),
                reason: "must be at least three times JOBS_HEARTBEAT_SECONDS".to_owned(),
            });
        }

        Ok(())
    }
}

impl QueueLaneConfig {
    fn from_env(prefix: &str, default_concurrency: usize) -> Result<Self, ConfigError> {
        let concurrency =
            env::non_zero_usize(&format!("{prefix}_CONCURRENCY"), default_concurrency)?;
        let claim_batch = env::non_zero_usize(&format!("{prefix}_CLAIM_BATCH"), concurrency)?;
        let config = Self {
            concurrency,
            claim_batch,
        };
        config.validate(prefix)?;
        Ok(config)
    }

    fn validate(&self, prefix: &str) -> Result<(), ConfigError> {
        if self.claim_batch > self.concurrency {
            return Err(ConfigError::Invalid {
                key: format!("{prefix}_CLAIM_BATCH"),
                reason: format!("must not exceed {prefix}_CONCURRENCY"),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_config() -> QueueConfig {
        let lane = QueueLaneConfig {
            concurrency: 8,
            claim_batch: 8,
        };
        QueueConfig {
            core_fast: lane.clone(),
            core_bulk: lane.clone(),
            ops: lane,
            poll_interval: Duration::from_millis(100),
            lease_duration: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(10),
            job_timeout: Duration::from_secs(60),
        }
    }

    fn duration_config() -> DurationConfig {
        DurationConfig {
            api_v2_url: "https://api-v2.soundcloud.com".parse().unwrap(),
            web_url: "https://soundcloud.com".parse().unwrap(),
            proxy_url: None,
            proxy_fallback: false,
            batch_size: 50,
            concurrency: 4,
            request_gap: Duration::from_millis(150),
            max_track_duration_ms: 420_000,
        }
    }

    #[test]
    fn lease_allows_three_heartbeat_intervals() {
        let mut config = queue_config();
        config.heartbeat_interval = Duration::from_secs(11);

        assert!(config.validate().is_err());
    }

    #[test]
    fn claim_batch_cannot_exceed_concurrency() {
        let mut config = queue_config();
        config.ops.claim_batch = config.ops.concurrency + 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn valid_queue_timing_is_accepted() {
        assert!(queue_config().validate().is_ok());
    }

    #[test]
    fn duration_batch_size_is_bounded() {
        let mut config = duration_config();
        config.batch_size = DURATION_MAX_BATCH_SIZE + 1;

        assert!(config.validate_limits().is_err());
    }

    #[test]
    fn duration_concurrency_is_bounded() {
        let mut config = duration_config();
        config.concurrency = DURATION_MAX_CONCURRENCY + 1;

        assert!(config.validate_limits().is_err());
    }

    #[test]
    fn duration_request_gap_has_a_safe_minimum() {
        let mut config = duration_config();
        config.request_gap = DURATION_MIN_REQUEST_GAP - Duration::from_millis(1);

        assert!(config.validate_limits().is_err());
    }

    #[test]
    fn snapshot_limit_must_fit_postgres_limit() {
        let config = SubscriptionsConfig {
            snapshot_dir: PathBuf::from("/snapshots"),
            max_file_bytes: 1_024,
            max_entries: usize::MAX,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn snapshot_file_has_a_useful_minimum_size() {
        let config = SubscriptionsConfig {
            snapshot_dir: PathBuf::from("/snapshots"),
            max_file_bytes: 1_023,
            max_entries: 1,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn oauth_app_rejects_blank_identity() {
        let result = OAuthAppBootstrap::new(
            "default".to_owned(),
            "   ".to_owned(),
            "secret".to_owned(),
            "https://localhost/callback".to_owned(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn oauth_app_id_is_stable_for_normalized_client_id() {
        let left = OAuthAppBootstrap::new(
            "default".to_owned(),
            " client ".to_owned(),
            "secret".to_owned(),
            "https://localhost/callback".to_owned(),
        )
        .unwrap();
        let right = OAuthAppBootstrap::new(
            "default".to_owned(),
            "client".to_owned(),
            "secret".to_owned(),
            "https://localhost/callback".to_owned(),
        )
        .unwrap();

        assert_eq!(left.id(), right.id());
    }

    #[test]
    fn network_batches_are_limited_to_bounded_waves() {
        assert!(validate_window("BATCH", 512, 128, 4).is_ok());
        assert!(validate_window("BATCH", 513, 128, 4).is_err());
    }

    #[test]
    fn sync_queue_lease_covers_the_full_refresh_and_retry_path() {
        let mut config = SyncQueueConfig {
            api_url: "https://api.soundcloud.com".parse().unwrap(),
            proxy_url: None,
            storage_url: "https://storage.example".parse().unwrap(),
            storage_token: "secret".to_owned().into(),
            concurrency: 16,
            claim_batch: 16,
            lease_duration: Duration::from_secs(299),
        };

        assert!(config.validate().is_err());
        config.lease_duration = Duration::from_secs(300);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn health_waits_only_for_worker_lanes_jobs_actually_feeds() {
        let mut dispatch = WorkerDispatchConfig {
            embed_lyrics: false,
            index_audio: false,
            transcribe: false,
            lyrics_align_rejected_retry_days: 30,
        };
        assert_eq!(
            dispatch.required_worker_lanes(),
            [WorkerLane::Encode, WorkerLane::Ai, WorkerLane::Collab]
        );

        dispatch.index_audio = true;
        dispatch.transcribe = true;
        assert_eq!(
            dispatch.required_worker_lanes(),
            [
                WorkerLane::Encode,
                WorkerLane::Ai,
                WorkerLane::Collab,
                WorkerLane::Audio,
                WorkerLane::Transcribe,
            ]
        );
    }
}
