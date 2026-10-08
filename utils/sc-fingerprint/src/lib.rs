use std::sync::Arc;

use dashmap::DashMap;
use once_cell::sync::Lazy;
use wreq::IntoEmulation;
use wreq_util::Profile;

mod legacy;

pub use wreq;

pub const DEFAULT_PROFILE: &str = "chrome_137";

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

const NAMES: &[&str] = &[
    "chrome_100",
    "chrome_101",
    "chrome_104",
    "chrome_105",
    "chrome_106",
    "chrome_107",
    "chrome_108",
    "chrome_109",
    "chrome_110",
    "chrome_114",
    "chrome_116",
    "chrome_117",
    "chrome_118",
    "chrome_119",
    "chrome_120",
    "chrome_123",
    "chrome_124",
    "chrome_126",
    "chrome_127",
    "chrome_128",
    "chrome_129",
    "chrome_130",
    "chrome_131",
    "chrome_132",
    "chrome_133",
    "chrome_134",
    "chrome_135",
    "chrome_136",
    "chrome_137",
    "edge_101",
    "edge_122",
    "edge_127",
    "edge_131",
    "edge_134",
    "firefox_109",
    "firefox_117",
    "firefox_128",
    "firefox_133",
    "firefox_135",
    "firefox_136",
    "firefox_139",
    "safari_16",
    "safari_18",
    "okhttp_5",
];

static CLIENTS: Lazy<DashMap<String, Arc<wreq::Client>>> = Lazy::new(DashMap::new);

pub fn default_profile() -> String {
    match std::env::var(PROFILE_ENV) {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => DEFAULT_PROFILE.to_string(),
    }
}

pub fn is_supported(profile: &str) -> bool {
    parse(profile).is_some()
}

pub fn supported_profiles() -> Vec<&'static str> {
    NAMES.iter().copied().filter(|n| is_supported(n)).collect()
}

fn parse(profile: &str) -> Option<Profile> {
    if !NAMES.contains(&profile) {
        return None;
    }
    serde_json::from_value(serde_json::Value::String(profile.to_string())).ok()
}

fn resolve(profile: Option<&str>) -> (String, Profile) {
    let requested = profile
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .unwrap_or_else(default_profile);

    if let Some(p) = parse(&requested) {
        return (requested, p);
    }

    tracing::warn!(
        requested = %requested,
        fallback = DEFAULT_PROFILE,
        "неизвестный профиль отпечатка, беру запасной"
    );
    let fallback = DEFAULT_PROFILE.to_string();
    let p = parse(&fallback).expect("встроенный профиль по умолчанию должен разбираться");
    (fallback, p)
}

pub fn emulation(profile: Option<&str>) -> (String, wreq::Emulation) {
    let (name, profile) = resolve(profile);
    let mut emulation = profile.into_emulation();
    legacy::restore(&name, &mut emulation);
    (name, emulation)
}

pub fn builder(profile: Option<&str>) -> wreq::ClientBuilder {
    let (_, emulation) = emulation(profile);
    wreq::Client::builder().emulation(emulation)
}

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
        assert!(is_supported(DEFAULT_PROFILE));
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
        assert!(Arc::ptr_eq(&a, &b));

        let c = client(Some("firefox_136")).expect("other profile");
        assert!(!Arc::ptr_eq(&a, &c));
    }

    #[test]
    fn every_advertised_profile_parses() {
        let names = supported_profiles();
        assert!(names.len() > 30);
        for name in names {
            assert!(parse(name).is_some(), "профиль {name} не разбирается");
        }
    }

    #[test]
    fn names_outside_the_advertised_list_fall_back() {
        for name in [
            "safari_17.0",
            "safari_ios_18.1.1",
            "okhttp_4.12",
            "okhttp_3.9",
        ] {
            assert!(!is_supported(name), "{name}");
            assert_eq!(emulation(Some(name)).0, DEFAULT_PROFILE, "{name}");
        }
    }
}
