use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::config::{AdmissionCfg, env_opt, env_str, env_u16, env_u32, env_u64};
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::synthetic::is_synthetic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Beta,
    All,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Beta => "beta",
            Access::All => "all",
        }
    }
}

#[derive(Clone)]
pub struct ChatConfig {
    pub enabled: bool,
    pub access: Access,
    pub beta_users: HashSet<String>,
    pub send_paused: bool,
    pub pg_pool_max: u32,
    pub max_parked: usize,
    pub hold_max_ms: u64,
    pub keys_dir: PathBuf,
    pub ock_current: u8,
    pub kek_current: u16,
    pub reports_kek_current: u16,
    pub loadtest: bool,
    pub loadtest_until: Option<DateTime<Utc>>,
    pub clock: fn() -> DateTime<Utc>,
    pub moderation: ModerationConfig,
    pub media: MediaConfig,
    pub starts: StartLimits,
}

#[derive(Clone, Debug)]
pub struct StartLimits {
    pub window: Duration,
    pub logins_per_client: u32,
    pub links_per_client: u32,
    pub trusted_proxies: Vec<ipnet::IpNet>,
}

impl StartLimits {
    pub fn from_admission(admission: &AdmissionCfg) -> Self {
        Self {
            window: admission.window,
            logins_per_client: admission.login.per_client,
            links_per_client: admission.link_create.per_client,
            trusted_proxies: admission.trusted_proxies.clone(),
        }
    }
}

pub const MODEL_TIMEOUT_DEFAULT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct ModerationConfig {
    pub enabled: bool,
    pub url: Option<String>,
    pub models: Vec<ModelChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub name: String,
    pub timeout: Duration,
}

impl ChatConfig {
    pub fn from_env(admission: &AdmissionCfg) -> Self {
        let keys_dir = PathBuf::from(env_str("CHAT_KEYS_DIR", "/run/secrets/chat"));
        Self {
            enabled: env_str("CHAT_ENABLED", "true") == "true",
            access: access(&env_str("CHAT_ACCESS", "beta")),
            beta_users: beta_users(&env_str("CHAT_BETA_USERS", "")),
            send_paused: env_str("CHAT_SEND_PAUSED", "false") == "true",
            pg_pool_max: env_u32("CHAT_PG_POOL_MAX", 8).max(1),
            max_parked: usize::try_from(env_u64("CHAT_MAX_PARKED", 30_000))
                .expect("CHAT_MAX_PARKED is too large"),
            hold_max_ms: env_u64("CHAT_HOLD_MAX_MS", 20_000),
            keys_dir: keys_dir.clone(),
            ock_current: u8::try_from(env_u16("CHAT_OCK_CURRENT", 1))
                .expect("CHAT_OCK_CURRENT is a version from 1 to 255"),
            kek_current: env_u16("CHAT_KEK_CURRENT", 1),
            reports_kek_current: env_u16("CHAT_REPORTS_KEK_CURRENT", 1),
            loadtest: env_str("CHAT_LOADTEST", "off") == "on",
            loadtest_until: loadtest_until(&env_str("CHAT_LOADTEST_UNTIL", "")),
            clock: Utc::now,
            moderation: ModerationConfig {
                enabled: env_str("CHAT_MOD_AI_ENABLED", "false") == "true",
                url: env_opt("CHAT_MOD_AI_URL"),
                models: models(
                    &env_str("CHAT_MOD_AI_MODEL", ""),
                    &env_str("CHAT_MOD_AI_TIMEOUT_S", ""),
                ),
            },
            media: MediaConfig::from_env(&keys_dir).unwrap_or_else(|error| panic!("{error}")),
            starts: StartLimits::from_admission(admission),
        }
    }

    pub fn loadtest_open(&self) -> bool {
        self.loadtest
            && self
                .loadtest_until
                .is_none_or(|until| (self.clock)() < until)
    }

    pub fn opens_for(&self, user_id: &str) -> bool {
        self.enabled
            && (self.access == Access::All
                || self.beta_users.contains(user_id)
                || (self.loadtest_open() && is_synthetic(user_id)))
    }
}

fn loadtest_until(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map(|at| at.with_timezone(&Utc))
        .ok()
        .or_else(|| DateTime::from_timestamp(raw.parse().ok()?, 0));
    Some(parsed.unwrap_or_else(|| {
        panic!("CHAT_LOADTEST_UNTIL must be an RFC 3339 time or epoch seconds, got {raw:?}")
    }))
}

fn access(raw: &str) -> Access {
    match raw.trim() {
        "all" => Access::All,
        _ => Access::Beta,
    }
}

fn models(names: &str, timeouts: &str) -> Vec<ModelChoice> {
    let mut timeouts = timeouts.split(',').map(str::trim);
    names
        .split(',')
        .map(|name| (name.trim(), timeouts.next()))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, timeout)| ModelChoice {
            name: name.to_owned(),
            timeout: timeout
                .and_then(|secs| secs.parse::<u64>().ok())
                .filter(|secs| *secs > 0)
                .map_or(MODEL_TIMEOUT_DEFAULT, Duration::from_secs),
        })
        .collect()
}

fn beta_users(raw: &str) -> HashSet<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;
