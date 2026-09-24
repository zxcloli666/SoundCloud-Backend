use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

const MAX_URL_LEN: usize = 4096;

const SOUNDCLOUD_PAGE_HOSTS: &[&str] = &[
    "soundcloud.com",
    "www.soundcloud.com",
    "m.soundcloud.com",
    "on.soundcloud.com",
];

const SOUNDCLOUD_API_HOSTS: &[&str] = &["api-v2.soundcloud.com", "api.soundcloud.com"];

const LOCAL_SUFFIXES: &[&str] = &[
    ".localhost",
    ".local",
    ".internal",
    ".home.arpa",
    ".localdomain",
];

pub fn soundcloud_page(url: &str) -> Option<Url> {
    let parsed = reachable(url)?;
    let named = parsed.host_str()?;
    (parsed.scheme() == "https" && SOUNDCLOUD_PAGE_HOSTS.contains(&named)).then_some(parsed)
}

pub fn soundcloud_api(url: &str) -> Option<Url> {
    let parsed = reachable(url)?;
    let named = parsed.host_str()?;
    (parsed.scheme() == "https" && SOUNDCLOUD_API_HOSTS.contains(&named)).then_some(parsed)
}

pub fn public_media(url: &str) -> Option<Url> {
    let parsed = reachable(url)?;
    matches!(parsed.scheme(), "http" | "https").then_some(parsed)
}

pub fn named_without_secrets(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return "<unreadable url>".to_owned();
    };
    parsed.set_query(None);
    parsed.set_fragment(None);
    let _ = parsed.set_password(None);
    let _ = parsed.set_username("");
    parsed.into()
}

fn reachable(url: &str) -> Option<Url> {
    if url.len() > MAX_URL_LEN {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let outside = match parsed.host()? {
        Host::Domain(name) => !names_our_own_network(name),
        Host::Ipv4(address) => routes_off_this_machine_v4(address),
        Host::Ipv6(address) => routes_off_this_machine_v6(address),
    };
    outside.then_some(parsed)
}

fn names_our_own_network(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost"
        || !name.contains('.')
        || LOCAL_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

fn routes_off_this_machine_v4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !(address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_multicast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..128).contains(&octets[1]))
        || octets[0] >= 240)
}

fn routes_off_this_machine_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return routes_off_this_machine_v4(mapped);
    }
    let segments = address.segments();
    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || segments[0] & 0xfe00 == 0xfc00
        || segments[0] & 0xffc0 == 0xfe80)
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod target_tests;
