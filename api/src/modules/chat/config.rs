use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::{env_opt, env_str, env_u16, env_u32, env_u64};

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
    pub moderation: ModerationConfig,
}

#[derive(Clone)]
pub struct ModerationConfig {
    pub enabled: bool,
    pub url: Option<String>,
    pub model: Option<String>,
}

impl ChatConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: env_str("CHAT_ENABLED", "true") == "true",
            access: access(&env_str("CHAT_ACCESS", "beta")),
            beta_users: beta_users(&env_str("CHAT_BETA_USERS", "")),
            send_paused: env_str("CHAT_SEND_PAUSED", "false") == "true",
            pg_pool_max: env_u32("CHAT_PG_POOL_MAX", 8).max(1),
            max_parked: usize::try_from(env_u64("CHAT_MAX_PARKED", 30_000))
                .expect("CHAT_MAX_PARKED is too large"),
            hold_max_ms: env_u64("CHAT_HOLD_MAX_MS", 20_000),
            keys_dir: PathBuf::from(env_str("CHAT_KEYS_DIR", "/run/secrets/chat")),
            ock_current: u8::try_from(env_u16("CHAT_OCK_CURRENT", 1))
                .expect("CHAT_OCK_CURRENT is a version from 1 to 255"),
            kek_current: env_u16("CHAT_KEK_CURRENT", 1),
            reports_kek_current: env_u16("CHAT_REPORTS_KEK_CURRENT", 1),
            loadtest: env_str("CHAT_LOADTEST", "off") == "on",
            moderation: ModerationConfig {
                enabled: env_str("CHAT_MOD_AI_ENABLED", "false") == "true",
                url: env_opt("CHAT_MOD_AI_URL"),
                model: env_opt("CHAT_MOD_AI_MODEL"),
            },
        }
    }

    pub fn opens_for(&self, user_id: &str) -> bool {
        self.enabled && (self.access == Access::All || self.beta_users.contains(user_id))
    }
}

fn access(raw: &str) -> Access {
    match raw.trim() {
        "all" => Access::All,
        _ => Access::Beta,
    }
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
