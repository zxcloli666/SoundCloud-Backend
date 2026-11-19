use std::path::PathBuf;
use std::time::Duration;

use backend_contracts::pipeline::MAX_VIDEO_CANDIDATES;
use base64::Engine as _;

use super::anthropic::{self, AnthropicLadder};
use super::env::{self, ConfigError};
use super::{http_url, positive};

const KEY_BYTES: usize = 32;

#[derive(Clone, Debug)]
pub struct ObjectsConfig {
    pub endpoint: url::Url,
    pub region: String,
    pub bucket: String,
    pub access_key_id: redact::Secret<String>,
    pub secret_access_key: redact::Secret<String>,
    pub path_style: bool,
}

impl ObjectsConfig {
    fn from_env() -> Result<Option<Self>, ConfigError> {
        let Some(endpoint) = env::optional("VIDEO_S3_ENDPOINT") else {
            return Ok(None);
        };
        Ok(Some(Self {
            endpoint: http_url("VIDEO_S3_ENDPOINT", endpoint)?,
            region: env::parse("VIDEO_S3_REGION", "us-east-1")?,
            bucket: env::required("VIDEO_S3_BUCKET")?,
            access_key_id: env::required("VIDEO_S3_ACCESS_KEY_ID")?.into(),
            secret_access_key: env::required("VIDEO_S3_SECRET_ACCESS_KEY")?.into(),
            path_style: env::boolean("VIDEO_S3_PATH_STYLE", true)?,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct VideoFindConfig {
    pub in_flight: i64,
    pub tick: Duration,
    pub timeout: Duration,
    pub account_pause: Duration,
    pub source_gap: Duration,
    pub youtube_pause: Duration,
    pub retry_base: Duration,
    pub max_attempts: u32,
    pub none_recheck: Duration,
    pub static_recheck: Duration,
    pub research_fresh: Duration,
    pub research_old: Duration,
    pub research_fresh_age: Duration,
    pub max_candidates: usize,
    pub download_timeout: Duration,
    pub stage_timeout: Duration,
    pub max_file_mib: u64,
    pub temp_ttl: Duration,
    pub vlm_backlog: i64,
    pub sweep_every: Duration,
    pub ytdlp: PathBuf,
    pub proxy_list: Option<PathBuf>,
    pub work_dir: PathBuf,
    pub objects: Option<ObjectsConfig>,
}

impl VideoFindConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let in_flight: i64 = env::parse("VIDEO_FIND_IN_FLIGHT", "0")?;
        let objects = ObjectsConfig::from_env()?;
        if in_flight > 0 && objects.is_none() {
            return Err(ConfigError::Missing("VIDEO_S3_ENDPOINT".to_owned()));
        }
        Ok(Self {
            in_flight,
            tick: Duration::from_secs(env::positive_u64("VIDEO_FIND_TICK_SECONDS", 30)?),
            timeout: Duration::from_secs(env::positive_u64("VIDEO_FIND_TIMEOUT_SECONDS", 4_200)?),
            account_pause: Duration::from_secs(env::positive_u64(
                "VIDEO_ACCOUNT_PAUSE_SECONDS",
                3_600,
            )?),
            source_gap: Duration::from_millis(env::positive_u64("VIDEO_SOURCE_GAP_MS", 1_500)?),
            youtube_pause: Duration::from_secs(env::positive_u64(
                "VIDEO_YOUTUBE_PAUSE_SECONDS",
                7,
            )?),
            retry_base: Duration::from_secs(env::positive_u64("VIDEO_RETRY_BASE_SECONDS", 900)?),
            max_attempts: positive("VIDEO_MAX_ATTEMPTS", env::parse("VIDEO_MAX_ATTEMPTS", "5")?)?,
            none_recheck: Duration::from_secs(
                env::positive_u64("VIDEO_NONE_RECHECK_DAYS", 30)? * 86_400,
            ),
            static_recheck: Duration::from_secs(
                env::positive_u64("VIDEO_STATIC_RECHECK_DAYS", 30)? * 86_400,
            ),
            research_fresh: Duration::from_secs(
                env::positive_u64("VIDEO_RESEARCH_FRESH_DAYS", 30)? * 86_400,
            ),
            research_old: Duration::from_secs(
                env::positive_u64("VIDEO_RESEARCH_OLD_DAYS", 90)? * 86_400,
            ),
            research_fresh_age: Duration::from_secs(
                env::positive_u64("VIDEO_RESEARCH_FRESH_AGE_DAYS", 180)? * 86_400,
            ),
            max_candidates: env::non_zero_usize(
                "VIDEO_MAX_CANDIDATES",
                MAX_VIDEO_CANDIDATES as usize,
            )?
            .min(MAX_VIDEO_CANDIDATES as usize),
            download_timeout: Duration::from_secs(env::positive_u64(
                "VIDEO_DOWNLOAD_TIMEOUT_SECONDS",
                240,
            )?),
            stage_timeout: Duration::from_secs(env::positive_u64(
                "VIDEO_STAGE_TIMEOUT_SECONDS",
                1_800,
            )?),
            max_file_mib: env::positive_u64("VIDEO_MAX_FILE_MIB", 96)?,
            temp_ttl: Duration::from_secs(env::positive_u64("VIDEO_TEMP_TTL_DAYS", 3)? * 86_400),
            vlm_backlog: i64::try_from(env::positive_u64("VIDEO_FIND_VLM_BACKLOG", 24)?)
                .unwrap_or(i64::MAX),
            sweep_every: Duration::from_secs(env::positive_u64("VIDEO_TEMP_SWEEP_SECONDS", 3_600)?),
            ytdlp: env::parse("VIDEO_YTDLP_PATH", "yt-dlp")?,
            proxy_list: env::optional("VIDEO_PROXY_LIST").map(PathBuf::from),
            work_dir: env::parse("VIDEO_WORK_DIR", "/tmp/video")?,
            objects,
        })
    }
}

#[derive(Clone, Debug)]
pub struct VideoVlmConfig {
    pub in_flight: i64,
    pub tick: Duration,
    pub timeout: Duration,
    pub month_usd: f64,
    pub recheck: Duration,
    pub verify_stored: i64,
    pub haiku: Option<AnthropicLadder>,
}

impl VideoVlmConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let in_flight: i64 = env::parse("VIDEO_VLM_IN_FLIGHT", "0")?;
        if in_flight > 0 && ObjectsConfig::from_env()?.is_none() {
            return Err(ConfigError::Missing("VIDEO_S3_ENDPOINT".to_owned()));
        }
        let month_usd = anthropic::month_usd("VIDEO_VLM_MONTH_USD", "0")?;
        let haiku = match in_flight > 0 && month_usd > 0.0 {
            true => anthropic::load("VIDEO_VLM_LADDER", None)?,
            false => None,
        };
        Ok(Self {
            in_flight,
            tick: Duration::from_secs(env::positive_u64("VIDEO_VLM_TICK_SECONDS", 30)?),
            timeout: Duration::from_secs(env::positive_u64("VIDEO_VLM_TIMEOUT_SECONDS", 4_200)?),
            month_usd,
            recheck: Duration::from_secs(env::positive_u64("VIDEO_VLM_RECHECK_SECONDS", 1_800)?),
            verify_stored: env::parse("VIDEO_VLM_VERIFY_STORED", "0")?,
            haiku,
        })
    }
}

#[derive(Clone, Debug)]
pub struct VideoAccountsConfig {
    pub check_every: Duration,
    pub spotify_totp_version: u32,
    pub spotify_totp_cipher: Vec<u8>,
    pub tg_webhook: redact::Secret<String>,
    pub key: redact::Secret<[u8; KEY_BYTES]>,
}

impl VideoAccountsConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let cipher = env::parse::<String>(
            "VIDEO_SPOTIFY_TOTP_CIPHER",
            "44,55,47,42,70,40,34,114,76,74,50,111,120,97,75,76,94,102,43,69,49,120,118,80,64,78",
        )?;
        let spotify_totp_cipher = cipher
            .split(',')
            .map(|byte| byte.trim().parse::<u8>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ConfigError::Invalid {
                key: "VIDEO_SPOTIFY_TOTP_CIPHER".to_owned(),
                reason: error.to_string(),
            })?;
        Ok(Self {
            check_every: Duration::from_secs(env::positive_u64(
                "VIDEO_ACCOUNTS_CHECK_SECONDS",
                86_400,
            )?),
            spotify_totp_version: env::parse("VIDEO_SPOTIFY_TOTP_VERSION", "61")?,
            spotify_totp_cipher,
            tg_webhook: env::optional("TG_WEBHOOK").unwrap_or_default().into(),
            key: account_key(&env::required("VIDEO_ACCOUNTS_KEY")?)?.into(),
        })
    }
}

fn account_key(encoded: &str) -> Result<[u8; KEY_BYTES], ConfigError> {
    let invalid = |reason: &str| ConfigError::Invalid {
        key: "VIDEO_ACCOUNTS_KEY".to_owned(),
        reason: reason.to_owned(),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| invalid("must be base64"))?;
    bytes
        .try_into()
        .map_err(|_| invalid("must decode to 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_key_is_32_bytes_of_base64() {
        let good = base64::engine::general_purpose::STANDARD.encode([7_u8; 32]);

        assert_eq!(account_key(&good).unwrap(), [7_u8; 32]);
        assert!(account_key("c2hvcnQ=").is_err());
        assert!(account_key("not base64!").is_err());
    }
}
