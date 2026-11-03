use redact::Secret;
use url::Url;

use crate::config::{env_opt, env_str};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Proxy,
    Local,
}

#[derive(Clone)]
pub struct RoomsConfig {
    pub mode: Mode,
    pub api: Option<Url>,
    pub token: Secret<String>,
}

impl RoomsConfig {
    pub fn from_env() -> Self {
        Self::parse(
            &env_str("ROOMS_MODE", "local"),
            env_opt("ROOMS_API").as_deref(),
            &env_str("INTERNAL_TOKEN", ""),
        )
    }

    pub fn parse(mode: &str, api: Option<&str>, token: &str) -> Self {
        let mode = match mode.trim() {
            "proxy" => Mode::Proxy,
            "" | "local" => Mode::Local,
            other => panic!("ROOMS_MODE must be proxy or local, got {other:?}"),
        };
        let api = api.map(str::trim).filter(|api| !api.is_empty()).map(|api| {
            let url = Url::parse(api).unwrap_or_else(|error| panic!("ROOMS_API is bad: {error}"));
            assert!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "ROOMS_API must be an http or https URL"
            );
            url
        });
        assert!(
            mode != Mode::Proxy || api.is_some(),
            "ROOMS_MODE=proxy needs ROOMS_API"
        );
        assert!(
            mode != Mode::Local || api.is_none() || !token.trim().is_empty(),
            "ROOMS_API is set, so INTERNAL_TOKEN must hold the admin token for the handoff"
        );
        Self {
            mode,
            api,
            token: token.trim().to_owned().into(),
        }
    }

    pub fn handoff_api(&self) -> Option<&Url> {
        match self.mode {
            Mode::Local => self.api.as_ref(),
            Mode::Proxy => None,
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
