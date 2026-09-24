use std::collections::BTreeSet;

use crate::source_tree::{read, sources_under};

const EXAMPLE: &str = ".env.example";
const TLS_COMMON: &str = "../utils/tls-common/src";

const READERS: [&str; 3] = ["env::var(\"", "env_bool(\"", "env_u16(\""];

const TEST_ONLY: [&str; 1] = ["STREAMING_SCHEMA_TEST_DATABASE_URL"];

fn settings_the_code_reads() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let files = sources_under("src")
        .into_iter()
        .chain(sources_under(TLS_COMMON));
    for (_, body) in files {
        for reader in READERS {
            let mut rest = body.as_str();
            while let Some(start) = rest.find(reader) {
                rest = &rest[start + reader.len()..];
                let Some(end) = rest.find('"') else {
                    break;
                };
                found.insert(rest[..end].to_string());
                rest = &rest[end..];
            }
        }
    }
    for exception in TEST_ONLY {
        found.remove(exception);
    }
    found
}

fn settings_the_example_promises() -> BTreeSet<String> {
    read(EXAMPLE)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_once('='))
        .map(|(key, _)| key.trim().to_string())
        .collect()
}

#[test]
fn every_setting_the_service_reads_is_written_down_in_the_example() {
    let read_by_code = settings_the_code_reads();
    assert!(
        read_by_code.contains("PORT") && read_by_code.len() > 20,
        "the scan found {} settings; it is no longer reading the service's code",
        read_by_code.len()
    );
    let promised = settings_the_example_promises();
    let missing: Vec<&String> = read_by_code.difference(&promised).collect();
    assert!(
        missing.is_empty(),
        "{EXAMPLE} says nothing about {missing:?}; whoever deploys this service \
         has no way to learn that these exist"
    );
}

#[test]
fn the_example_promises_no_setting_the_service_ignores() {
    let promised = settings_the_example_promises();
    let read_by_code = settings_the_code_reads();
    let ignored: Vec<&String> = promised.difference(&read_by_code).collect();
    assert!(
        ignored.is_empty(),
        "{EXAMPLE} offers {ignored:?}, which nothing reads; setting them changes nothing \
         and hides that the real knob is gone"
    );
}
