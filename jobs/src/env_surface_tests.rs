use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const EXAMPLE: &str = ".env.example";

const READERS: &[&str] = &[
    "env::optional",
    "env::required",
    "env::parse",
    "env::positive_u64",
    "env::non_zero_usize",
    "env::boolean",
    "std::env::var",
    "schedule_interval(",
    "stream_bytes(",
];

const DATABASE_PREFIXES: &[&str] = &["", "OPS_"];
const LANE_PREFIXES: &[&str] = &["JOBS_CORE_FAST", "JOBS_CORE_BULK", "JOBS_OPS"];
const LANE_SETTINGS: &[&str] = &["_CONCURRENCY", "_CLAIM_BATCH"];

const READ_BY_A_LIBRARY: &[&str] = &[
    "RUST_LOG",
    "HOSTNAME",
    "DATABASE_HOST",
    "DATABASE_PORT",
    "DATABASE_NAME",
    "DATABASE_USERNAME",
    "DATABASE_PASSWORD",
    "DATABASE_SSL_MODE",
    "DATABASE_SSL_CA",
    "DATABASE_SSL_CERT",
    "DATABASE_SSL_KEY",
    "OPS_DATABASE_HOST",
    "OPS_DATABASE_PORT",
    "OPS_DATABASE_NAME",
    "OPS_DATABASE_USERNAME",
    "OPS_DATABASE_PASSWORD",
    "OPS_DATABASE_SSL_MODE",
    "OPS_DATABASE_SSL_CA",
    "OPS_DATABASE_SSL_CERT",
    "OPS_DATABASE_SSL_KEY",
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
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".rs")
                && !name.contains("test")
                && let Ok(body) = fs::read_to_string(&path)
            {
                bodies.push(body);
            }
        }
    }
    bodies.join("\n")
}

fn names_a_setting(literal: &str) -> bool {
    literal.len() >= 3
        && literal.starts_with(|c: char| c.is_ascii_uppercase())
        && literal
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn first_literal_after(rest: &str) -> Option<(String, usize)> {
    let open = rest.find('"')?;
    let after = &rest[open + 1..];
    let close = after.find('"')?;
    Some((after[..close].to_owned(), open + 1 + close + 1))
}

fn settings_the_code_reads() -> BTreeSet<String> {
    let sources = every_source();
    let mut found = BTreeSet::new();

    for reader in READERS {
        let mut rest = sources.as_str();
        while let Some(start) = rest.find(reader) {
            rest = &rest[start + reader.len()..];
            let Some((name, consumed)) = first_literal_after(rest) else {
                break;
            };
            if !names_a_setting(&name) {
                rest = &rest[consumed..];
                continue;
            }
            let prefixed = rest[..consumed].contains("key(");
            if prefixed {
                for prefix in DATABASE_PREFIXES {
                    found.insert(format!("{prefix}{name}"));
                }
            } else {
                found.insert(name);
            }
            rest = &rest[consumed..];
        }
    }

    for lane in LANE_PREFIXES {
        for setting in LANE_SETTINGS {
            found.insert(format!("{lane}{setting}"));
        }
    }
    for named in READ_BY_A_LIBRARY {
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
fn the_example_promises_no_setting_the_worker_ignores() {
    let ignored: Vec<String> = settings_the_example_promises()
        .difference(&settings_the_code_reads())
        .cloned()
        .collect();

    assert!(
        ignored.is_empty(),
        "these settings are offered to whoever fills the file in, and nothing reads them; a \
         setting that quietly does nothing is worse than a missing one, because it reads as \
         configured: {ignored:?}"
    );
}

#[test]
fn every_setting_the_worker_reads_is_written_down_in_the_example() {
    let promised = settings_the_example_promises();
    let undocumented: Vec<String> = settings_the_code_reads()
        .difference(&promised)
        .filter(|name| !READ_BY_A_LIBRARY.contains(&name.as_str()))
        .cloned()
        .collect();

    assert!(
        undocumented.is_empty(),
        "the worker changes behaviour on these, and whoever deploys it has no way to learn \
         they exist: {undocumented:?}"
    );
}

#[test]
fn the_scan_finds_the_settings_it_claims_to_find() {
    let read = settings_the_code_reads();
    assert!(
        read.len() >= 40,
        "only {} settings were recognised; the reader list has gone stale and this guard is \
         passing on an empty scan",
        read.len()
    );
    for known in [
        "INTERNAL_TOKEN",
        "STORAGE_TOKEN",
        "NATS_URL",
        "PG_POOL_MAX",
        "OPS_DATABASE_URL",
        "JOBS_CORE_FAST_CONCURRENCY",
    ] {
        assert!(read.contains(known), "{known} must be recognised as read");
    }
}
