use std::env;

use stream_ticket::{StreamTicketKey, StreamTicketKeys};

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub database_host: String,
    pub database_port: u16,
    pub database_username: String,
    pub database_password: String,
    pub database_name: String,
    pub database_ssl_ca: Option<String>,
    pub database_ssl_cert: Option<String>,
    pub database_ssl_key: Option<String>,
    pub database_pool_max: usize,
    pub database_acquire_timeout_secs: u64,
    pub sc_proxy_url: String,
    pub sc_proxy_fallback: bool,
    pub sc_oauth_fallback_sessions: usize,
    pub sc_cookies: Vec<String>,
    pub premium_only: bool,
    pub storage_url: String,
    pub storage_public_url: String,
    pub storage_upload_url: String,
    pub storage_token: String,
    pub storage_cleanup_days: u64,
    pub storage_max_size_bytes: u64,
    pub storage_cleanup_interval_secs: u64,
    pub internal_token: String,
    pub stream_ticket_keys: StreamTicketKeys,
    pub decrypt_device: Option<String>,
    pub edge_wvd_dir: Option<String>,
    pub edge_wvd_token: Option<String>,
    pub edge_wvd_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let cookies = parse_cookie_list(&env::var("SC_COOKIES").unwrap_or_default());

        let storage_url = env::var("STORAGE_URL").unwrap_or_default();
        let storage_public_url = env::var("STORAGE_PUBLIC_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| storage_url.clone());
        let storage_upload_url = env::var("STORAGE_UPLOAD_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| storage_url.clone());

        Self {
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080),
            database_host: env::var("DATABASE_HOST").unwrap_or_else(|_| "localhost".into()),
            database_port: env::var("DATABASE_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5432),
            database_username: env::var("DATABASE_USERNAME")
                .unwrap_or_else(|_| "soundcloud".into()),
            database_password: env::var("DATABASE_PASSWORD")
                .unwrap_or_else(|_| "soundcloud".into()),
            database_name: env::var("DATABASE_NAME")
                .unwrap_or_else(|_| "soundcloud_desktop".into()),
            database_ssl_ca: env::var("DATABASE_SSL_CA").ok().filter(|v| !v.is_empty()),
            database_ssl_cert: env::var("DATABASE_SSL_CERT").ok().filter(|v| !v.is_empty()),
            database_ssl_key: env::var("DATABASE_SSL_KEY").ok().filter(|v| !v.is_empty()),
            database_pool_max: pool_max_from_env(env::var("PG_POOL_MAX").ok().as_deref()),
            database_acquire_timeout_secs: acquire_timeout_from_env(
                env::var("PG_ACQUIRE_TIMEOUT_SECS").ok().as_deref(),
            ),
            sc_proxy_url: env::var("SC_PROXY_URL").unwrap_or_default(),
            sc_proxy_fallback: env::var("SC_PROXY_FALLBACK")
                .map(|v| v == "true")
                .unwrap_or(false),
            sc_oauth_fallback_sessions: env::var("SC_OAUTH_FALLBACK_SESSIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            sc_cookies: cookies,
            premium_only: env::var("PREMIUM_ONLY")
                .map(|v| v == "true")
                .unwrap_or(false),
            storage_url,
            storage_public_url,
            storage_upload_url,
            storage_token: env::var("STORAGE_TOKEN").unwrap_or_default(),
            storage_cleanup_days: env::var("STORAGE_CLEANUP_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(7),
            storage_max_size_bytes: env::var("STORAGE_MAX_SIZE_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            storage_cleanup_interval_secs: env::var("STORAGE_CLEANUP_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            internal_token: env::var("INTERNAL_TOKEN").unwrap_or_default(),
            stream_ticket_keys: stream_ticket_keys(),
            decrypt_device: env::var("SC_DECRYPT_DEVICE").ok().filter(|s| !s.is_empty()),
            edge_wvd_dir: env::var("SC_EDGE_WVD_DIR").ok().filter(|s| !s.is_empty()),
            edge_wvd_token: env::var("SC_EDGE_WVD_TOKEN").ok().filter(|s| !s.is_empty()),
            edge_wvd_url: env::var("SC_EDGE_WVD_URL").ok().filter(|s| !s.is_empty()),
        }
    }

    pub fn storage_enabled(&self) -> bool {
        !self.storage_url.is_empty() && !self.storage_token.is_empty()
    }

    pub fn cookies_enabled(&self) -> bool {
        self.sc_cookies
            .iter()
            .any(|c| parse_cookie_value(c, "oauth_token").is_some())
    }
}

fn stream_ticket_keys() -> StreamTicketKeys {
    let active = env::var("STREAM_TICKET_KEY").expect("STREAM_TICKET_KEY must be set");
    let previous = env::var("STREAM_TICKET_PREVIOUS_KEYS").unwrap_or_default();
    let keys = std::iter::once(active.as_str())
        .chain(
            previous
                .split(',')
                .map(str::trim)
                .filter(|key| !key.is_empty()),
        )
        .map(|key| {
            StreamTicketKey::from_base64(key)
                .expect("stream ticket keys must be base64-encoded 32 bytes")
        })
        .collect();
    StreamTicketKeys::new(keys).expect("stream ticket key ring contains too many keys")
}

fn parse_cookie_list(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn parse_cookie_value(cookies: &str, name: &str) -> Option<String> {
    for part in cookies.split(';') {
        let part = part.trim();
        if let Some(idx) = part.find('=') {
            let key = part[..idx].trim();
            if key == name {
                let val = part[idx + 1..].trim();
                return Some(urlencoding_decode(val));
            }
        }
    }
    None
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().unwrap_or(b'0');
            let lo = chars.next().unwrap_or(b'0');
            let val = hex_digit(hi).unwrap_or(0) * 16 + hex_digit(lo).unwrap_or(0);
            result.push(val as char);
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

const DEFAULT_POOL_MAX: usize = 8;
const MAX_POOL_MAX: usize = 64;
const DEFAULT_ACQUIRE_TIMEOUT_SECS: u64 = 10;
const MAX_ACQUIRE_TIMEOUT_SECS: u64 = 60;

fn pool_max_from_env(raw: Option<&str>) -> usize {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_POOL_MAX)
        .min(MAX_POOL_MAX)
}

fn acquire_timeout_from_env(raw: Option<&str>) -> u64 {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ACQUIRE_TIMEOUT_SECS)
        .min(MAX_ACQUIRE_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pool_nobody_configured_is_small_and_explicit() {
        assert_eq!(pool_max_from_env(None), DEFAULT_POOL_MAX);
        assert_eq!(pool_max_from_env(Some("")), DEFAULT_POOL_MAX);
        assert_eq!(pool_max_from_env(Some("   ")), DEFAULT_POOL_MAX);
    }

    #[test]
    fn a_configured_pool_is_taken_as_asked() {
        assert_eq!(pool_max_from_env(Some("4")), 4);
        assert_eq!(pool_max_from_env(Some(" 12 ")), 12);
    }

    #[test]
    fn nonsense_falls_back_instead_of_opening_an_unbounded_pool() {
        for raw in ["0", "-1", "many", "8.5"] {
            assert_eq!(
                pool_max_from_env(Some(raw)),
                DEFAULT_POOL_MAX,
                "{raw} must not become a pool size"
            );
        }
    }

    #[test]
    fn a_pool_larger_than_the_server_can_afford_is_capped() {
        assert_eq!(
            pool_max_from_env(Some("1000")),
            MAX_POOL_MAX,
            "PostgreSQL runs with max_connections=80 shared across every service"
        );
    }

    #[test]
    fn waiting_for_a_connection_is_bounded_even_when_nobody_configured_it() {
        assert_eq!(acquire_timeout_from_env(None), DEFAULT_ACQUIRE_TIMEOUT_SECS);
        assert_eq!(
            acquire_timeout_from_env(Some("")),
            DEFAULT_ACQUIRE_TIMEOUT_SECS
        );
    }

    #[test]
    fn a_configured_wait_is_taken_as_asked() {
        assert_eq!(acquire_timeout_from_env(Some("3")), 3);
        assert_eq!(acquire_timeout_from_env(Some(" 30 ")), 30);
    }

    #[test]
    fn zero_and_nonsense_do_not_turn_into_waiting_forever() {
        for raw in ["0", "-1", "forever", "10.5"] {
            assert_eq!(
                acquire_timeout_from_env(Some(raw)),
                DEFAULT_ACQUIRE_TIMEOUT_SECS,
                "{raw} must not disable the wait budget"
            );
        }
    }

    #[test]
    fn a_wait_longer_than_a_client_will_ever_sit_there_is_capped() {
        assert_eq!(
            acquire_timeout_from_env(Some("86400")),
            MAX_ACQUIRE_TIMEOUT_SECS
        );
    }
}
