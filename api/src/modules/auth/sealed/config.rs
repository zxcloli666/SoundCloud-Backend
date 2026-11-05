use std::path::PathBuf;

use ipnet::IpNet;

use crate::config::{env_opt, env_str, env_u16, env_u32};

#[derive(Clone, Debug)]
pub struct SealedCfg {
    pub enabled: bool,
    pub plain: bool,
    pub pow: bool,
    pub keys_dir: PathBuf,
    pub ock_current: u8,
    pub node: String,
    pub free_per_min: u32,
    pub pow_per_min: u32,
    pub pow_floor: u32,
    pub pow_cap: u32,
    pub trusted_proxies: Vec<IpNet>,
}

impl SealedCfg {
    pub(crate) fn from_env(trusted_proxies: Vec<IpNet>) -> Self {
        let pow_floor = env_u32("AUTH_POW_FLOOR", 100);
        let pow_cap = env_u32("AUTH_POW_CAP", 6000);
        assert!(
            (1..=pow_cap).contains(&pow_floor),
            "AUTH_POW_FLOOR must be from 1 to AUTH_POW_CAP"
        );
        Self {
            enabled: switch(&env_str("AUTH_X", "off")),
            plain: switch(&env_str("AUTH_X_PLAIN", "off")),
            pow: switch(&env_str("AUTH_POW", "off")),
            keys_dir: PathBuf::from(env_str("AUTH_KEYS_DIR", "/run/secrets/chat")),
            ock_current: u8::try_from(env_u16("AUTH_OCK_CURRENT", 1))
                .expect("AUTH_OCK_CURRENT is a version from 1 to 255"),
            node: env_opt("SCD_HOST").unwrap_or_else(|| "local".to_owned()),
            free_per_min: env_u32("AUTH_FREE_PER_MIN", 120),
            pow_per_min: env_u32("AUTH_POW_PER_MIN", 1000),
            pow_floor,
            pow_cap,
            trusted_proxies,
        }
    }
}

fn switch(value: &str) -> bool {
    matches!(value, "on" | "1" | "true")
}
