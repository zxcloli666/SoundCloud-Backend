use crate::config::{
    AdmissionCfg, DatabaseCfg, SoundcloudCfg, database_from_env, env_str, env_u16,
};
use crate::modules::chat::ChatConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Full,
    Rt,
}

pub fn role() -> Role {
    match std::env::var("API_ROLE").unwrap_or_default().trim() {
        "" | "full" => Role::Full,
        "rt" => Role::Rt,
        other => panic!("API_ROLE must be rt or full, got {other:?}"),
    }
}

pub struct RtConfig {
    pub port: u16,
    pub database: DatabaseCfg,
    pub redis_url: String,
    pub redis_rt_url: String,
    pub admission: AdmissionCfg,
    pub soundcloud: SoundcloudCfg,
    pub chat: ChatConfig,
}

impl RtConfig {
    pub fn from_env() -> Self {
        Self {
            port: env_u16("PORT", 3000),
            database: database_from_env("", Some("localhost"))
                .expect("core database config: default host is always present"),
            redis_url: env_str("REDIS_URL", "redis://localhost:6379"),
            redis_rt_url: env_str("REDIS_RT_URL", "redis://localhost:6379"),
            admission: AdmissionCfg::from_env(),
            soundcloud: SoundcloudCfg::from_env(),
            chat: ChatConfig::from_env(),
        }
    }
}
