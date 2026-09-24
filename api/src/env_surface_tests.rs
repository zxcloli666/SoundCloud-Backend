use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const EXAMPLE: &str = ".env.example";

const READERS: &[&str] = &[
    "env_str(\"",
    "env_opt(\"",
    "env_u16(\"",
    "env_u32(\"",
    "env_u64(\"",
    "env_usize(\"",
    "env_f64(\"",
    "admission_value(\"",
    "admission_u32(\"",
    "std::env::var(\"",
];

const READ_THROUGH_A_PREFIX: &[&str] = &[
    "AUTH_LOGIN_PER_CLIENT",
    "AUTH_LOGIN_GLOBAL",
    "AUTH_LINK_CREATE_PER_CLIENT",
    "AUTH_LINK_CREATE_GLOBAL",
    "RESOLVE_PER_CLIENT",
    "RESOLVE_GLOBAL",
];

const READ_BY_A_LIBRARY: &[&str] = &[
    "DATABASE_URL",
    "DATABASE_HOST",
    "DATABASE_PORT",
    "DATABASE_NAME",
    "DATABASE_USERNAME",
    "DATABASE_PASSWORD",
    "DATABASE_SSL_MODE",
    "DATABASE_SSL_CA",
    "DATABASE_SSL_CERT",
    "DATABASE_SSL_KEY",
    "OPS_DATABASE_URL",
    "PG_POOL_MAX",
    "PG_ACQUIRE_TIMEOUT_SECS",
    "RUST_LOG",
    "TLS_PROXY_PROTOCOL",
    "TLS_PROXY_TRUSTED_CIDRS",
    "TLS_PROXY_TRUSTED_HOSTS",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn every_source() -> String {
    let mut bodies = Vec::new();
    let mut stack = vec![crate_root().join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|name| name.to_str()) == Some("rs")
                && let Ok(body) = fs::read_to_string(&path)
            {
                bodies.push(body);
            }
        }
    }
    bodies.join("\n")
}

fn settings_the_code_reads() -> BTreeSet<String> {
    let sources = every_source();
    let mut found = BTreeSet::new();
    for reader in READERS {
        let mut rest = sources.as_str();
        while let Some(start) = rest.find(reader) {
            rest = &rest[start + reader.len()..];
            let Some(end) = rest.find('"') else {
                break;
            };
            found.insert(rest[..end].to_owned());
            rest = &rest[end..];
        }
    }
    for named in READ_THROUGH_A_PREFIX.iter().chain(READ_BY_A_LIBRARY) {
        found.insert((*named).to_owned());
    }
    found
}

fn settings_the_example_promises() -> BTreeSet<String> {
    let example = fs::read_to_string(crate_root().join(EXAMPLE)).expect("the example is readable");
    example
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name.trim().to_owned())
        .filter(|name| !name.is_empty() && !name.starts_with('#'))
        .collect()
}

#[test]
fn the_example_promises_no_setting_the_service_ignores() {
    let promised = settings_the_example_promises();
    let read = settings_the_code_reads();
    let ignored: Vec<&String> = promised.difference(&read).collect();

    assert!(
        ignored.is_empty(),
        "these settings are offered to whoever fills the file in, and nothing reads them; a \
         setting that quietly does nothing is worse than a missing one, because it reads as \
         configured: {ignored:?}"
    );
}

#[test]
fn every_setting_the_service_reads_is_written_down_in_the_example() {
    let promised = settings_the_example_promises();
    let read = settings_the_code_reads();
    let undocumented: Vec<&String> = read
        .difference(&promised)
        .filter(|name| !READ_BY_A_LIBRARY.contains(&name.as_str()))
        .collect();

    assert!(
        undocumented.is_empty(),
        "the service changes behaviour on these, and whoever deploys it has no way to learn \
         they exist: {undocumented:?}"
    );
}

#[test]
fn the_scan_finds_the_settings_it_claims_to_find() {
    let read = settings_the_code_reads();
    assert!(
        read.len() >= 30,
        "only {} settings were recognised; the reader list has gone stale and this guard is \
         passing on an empty scan",
        read.len()
    );
    for known in ["ADMIN_TOKEN", "REDIS_URL", "QDRANT_URL"] {
        assert!(read.contains(known), "{known} must be recognised as read");
    }
}
