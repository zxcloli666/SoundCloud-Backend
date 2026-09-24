use std::fs;
use std::path::{Path, PathBuf};

const OWNER_TOKEN_READS: &[&str] = &["api_get::<", "api_get_value(", "api_get_absolute_value("];

const ALLOWED: &[(&str, &str)] = &[
    (
        "src/modules/auth/service/login.rs",
        "the owner's own /me during the OAuth login exchange",
    ),
    (
        "src/modules/playlists/service.rs",
        "a private playlist the caller opened with a secret token",
    ),
    (
        "src/modules/tracks/service.rs",
        "a private track the caller opened with a secret token",
    ),
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn serving_sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut stack = vec![crate_root().join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !name.ends_with(".rs") || name.contains("test") {
                continue;
            }
            if let Ok(body) = fs::read_to_string(&path) {
                found.push((shown(&path), body));
            }
        }
    }
    found.sort();
    found
}

fn shown(path: &Path) -> String {
    path.strip_prefix(crate_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_the_transport_itself(file: &str) -> bool {
    file.starts_with("src/sc/")
}

#[test]
fn only_the_named_flows_spend_the_listeners_soundcloud_token() {
    let mut unexpected: Vec<String> = Vec::new();

    for (file, body) in serving_sources() {
        if is_the_transport_itself(&file) || ALLOWED.iter().any(|(named, _)| *named == file) {
            continue;
        }
        for (number, line) in body.lines().enumerate() {
            if OWNER_TOKEN_READS.iter().any(|call| line.contains(call)) {
                unexpected.push(format!("{file}:{}: {}", number + 1, line.trim()));
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "a read with the listener's own token is a control or private flow, and every one of \
         them is named in this list; anything else belongs behind `ScReadService`, which goes \
         out through the relay and survives a ban:\n  {}",
        unexpected.join("\n  ")
    );
}

#[test]
fn every_named_flow_still_makes_the_call_it_was_allowed_for() {
    let sources = serving_sources();
    let mut silent: Vec<&str> = Vec::new();

    for (file, why) in ALLOWED {
        let Some((_, body)) = sources.iter().find(|(named, _)| named == file) else {
            silent.push(file);
            continue;
        };
        if !OWNER_TOKEN_READS.iter().any(|call| body.contains(call)) {
            silent.push(why);
        }
    }

    assert!(
        silent.is_empty(),
        "these entries excuse a call that is no longer made, so the list now excuses whatever \
         takes their place: {silent:?}"
    );
}
