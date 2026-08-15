//! Browser TLS/HTTP2 fingerprints for every outgoing request we make to somebody
//! else's service.
//!
//! # Why this exists
//!
//! Anti-bot systems fingerprint the TLS handshake (JA3/JA4) and the HTTP/2 SETTINGS
//! order **before** a single header is read. `rustls` — what plain `reqwest` uses —
//! has one recognisable shape, so a `User-Agent: Chrome` on a rustls handshake is a
//! contradiction that reads as "bot" instantly. A residential IP does not save it.
//!
//! Measured against Cian from one residential Rostelecom address, same second, same
//! headers: the rustls fingerprint `t13d1011h2_61a7ad8aa9b6_…` got a captcha redirect,
//! a Chrome fingerprint `t13d1516h2_8daaf6152771_…` got the page. The transport, not
//! the address, was the tell.
//!
//! # How to use it
//!
//! Two entry points, depending on whether you need your own client settings:
//!
//! ```no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Just give me a client (cached per profile — building one is expensive).
//! let http = sc_fingerprint::client(None)?;
//! let body = http.get("https://api-v2.soundcloud.com/tracks").send().await?.text().await?;
//!
//! // 2. Give me a builder so I can add my own timeouts / proxy / redirect policy.
//! let mine = sc_fingerprint::builder(Some("firefox_139"))
//!     .connect_timeout(std::time::Duration::from_secs(10))
//!     .build()?;
//! # let _ = (body, mine);
//! # Ok(())
//! # }
//! ```
//!
//! # Choosing the profile
//!
//! Precedence: explicit argument → `SC_IMPERSONATE` env → [`DEFAULT_PROFILE`].
//! An unknown name never fails a request — it logs a warning and falls back to the
//! default, because a typo in a config file must not take a fleet offline.
//!
//! Keep [`DEFAULT_PROFILE`] near the current stable Chrome. A fingerprint of a
//! browser nobody runs any more is as suspicious as a bot's.

use std::sync::Arc;

use dashmap::DashMap;
use once_cell::sync::Lazy;

pub use wreq;
pub use wreq_util::Emulation;

/// The profile used when nothing else is specified. Bump it as Chrome moves; the
/// point is to look like traffic that actually exists in the wild today.
pub const DEFAULT_PROFILE: &str = "chrome_137";

/// Environment variable that overrides [`DEFAULT_PROFILE`] process-wide. Lets a whole
/// fleet move to a new browser without a rebuild.
pub const PROFILE_ENV: &str = "SC_IMPERSONATE";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("не удалось собрать клиент с профилем {profile}: {source}")]
    Build {
        profile: String,
        #[source]
        source: wreq::Error,
    },
}

static CLIENTS: Lazy<DashMap<String, Arc<wreq::Client>>> = Lazy::new(DashMap::new);

/// Profile name in effect when a caller does not name one.
pub fn default_profile() -> String {
    match std::env::var(PROFILE_ENV) {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => DEFAULT_PROFILE.to_string(),
    }
}

/// Whether a profile name is one this build knows.
pub fn is_supported(profile: &str) -> bool {
    parse(profile).is_some()
}

/// Every profile this build can emulate, for diagnostics and admin UIs.
pub fn supported_profiles() -> Vec<&'static str> {
    const NAMES: &[&str] = &[
        "chrome_100", "chrome_101", "chrome_104", "chrome_105", "chrome_106",
        "chrome_107", "chrome_108", "chrome_109", "chrome_110", "chrome_114",
        "chrome_116", "chrome_117", "chrome_118", "chrome_119", "chrome_120",
        "chrome_123", "chrome_124", "chrome_126", "chrome_127", "chrome_128",
        "chrome_129", "chrome_130", "chrome_131", "chrome_132", "chrome_133",
        "chrome_134", "chrome_135", "chrome_136", "chrome_137",
        "edge_101", "edge_122", "edge_127", "edge_131", "edge_134",
        "firefox_109", "firefox_117", "firefox_128", "firefox_133", "firefox_135",
        "firefox_136", "firefox_139",
        "safari_16", "safari_18", "okhttp_5",
    ];
    NAMES.iter().copied().filter(|n| is_supported(n)).collect()
}

fn parse(profile: &str) -> Option<Emulation> {
    serde_json::from_value(serde_json::Value::String(profile.to_string())).ok()
}

/// Resolve a profile name to an emulation, falling back to the default (and warning)
/// when the name is unknown. Never fails: a bad name must not break traffic.
pub fn emulation(profile: Option<&str>) -> (String, Emulation) {
    let requested = profile
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .unwrap_or_else(default_profile);

    if let Some(e) = parse(&requested) {
        return (requested, e);
    }

    tracing::warn!(
        requested = %requested,
        fallback = DEFAULT_PROFILE,
        "неизвестный профиль отпечатка, беру запасной"
    );
    let fallback = DEFAULT_PROFILE.to_string();
    let e = parse(&fallback).expect("встроенный профиль по умолчанию должен разбираться");
    (fallback, e)
}

/// A client builder already carrying the fingerprint. Use when you need your own
/// timeouts, redirect policy or proxy on top.
pub fn builder(profile: Option<&str>) -> wreq::ClientBuilder {
    let (_, emulation) = emulation(profile);
    wreq::Client::builder().emulation(emulation)
}

/// A ready client for `profile`, cached per profile.
///
/// Building a client compiles a fresh BoringSSL configuration, which costs more than
/// most requests it would serve — so callers on the hot path should take the cached
/// one rather than build their own.
pub fn client(profile: Option<&str>) -> Result<Arc<wreq::Client>, Error> {
    let (name, emulation) = emulation(profile);

    if let Some(existing) = CLIENTS.get(&name) {
        return Ok(existing.clone());
    }

    let built = wreq::Client::builder()
        .emulation(emulation)
        .build()
        .map_err(|source| Error::Build {
            profile: name.clone(),
            source,
        })?;

    let built = Arc::new(built);
    CLIENTS.insert(name, built.clone());
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_real() {
        assert!(is_supported(DEFAULT_PROFILE), "дефолтный профиль должен существовать");
    }

    #[test]
    fn explicit_profile_wins() {
        let (name, _) = emulation(Some("firefox_139"));
        assert_eq!(name, "firefox_139");
    }

    #[test]
    fn unknown_profile_falls_back_and_does_not_fail() {
        let (name, _) = emulation(Some("netscape_navigator"));
        assert_eq!(name, DEFAULT_PROFILE);
        assert!(client(Some("netscape_navigator")).is_ok());
    }

    #[test]
    fn blank_profile_is_treated_as_unset() {
        let (name, _) = emulation(Some("   "));
        assert_eq!(name, default_profile());
    }

    #[test]
    fn clients_are_cached_per_profile() {
        let a = client(Some("chrome_133")).expect("client");
        let b = client(Some("chrome_133")).expect("cached client");
        assert!(Arc::ptr_eq(&a, &b), "один профиль — один клиент");

        let c = client(Some("firefox_136")).expect("other profile");
        assert!(!Arc::ptr_eq(&a, &c), "разные профили — разные клиенты");
    }

    #[test]
    fn every_advertised_profile_parses() {
        let names = supported_profiles();
        assert!(names.len() > 30, "список профилей подозрительно короткий");
        for name in names {
            assert!(parse(name).is_some(), "профиль {name} не разбирается");
        }
    }
}
