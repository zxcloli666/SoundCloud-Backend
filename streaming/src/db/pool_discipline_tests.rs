use crate::source_tree::{read, sources};

const POOL_OWNER: &str = "src/db/postgres.rs";
const SENTINEL: &str = "src/db/pool_discipline_tests.rs";

const ALLOWED_USE_ROOTS: [&str; 10] = [
    "crate",
    "deadpool_postgres",
    "rand",
    "rustls",
    "std",
    "super",
    "tokio_postgres",
    "tokio_postgres_rustls",
    "tracing",
    "uuid",
];

const NETWORK_TOKENS: [&str; 8] = [
    "wreq",
    "reqwest",
    "call_relay",
    "axum",
    "crate::stream",
    "crate::sc_methods",
    "AnonClient",
    "StorageClient",
];

const ESCAPING_TYPES: [&str; 3] = ["Object", "Client", "Transaction"];

fn use_roots(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("use "))
        .map(|rest| {
            rest.trim_start_matches("::")
                .split(|character: char| !(character.is_alphanumeric() || character == '_'))
                .find(|token| !token.is_empty())
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

fn public_signatures(body: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let mut open: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if open.is_none()
            && (trimmed.starts_with("pub fn ") || trimmed.starts_with("pub async fn "))
        {
            open = Some(String::new());
        }
        let Some(signature) = open.as_mut() else {
            continue;
        };
        let end = trimmed.find('{').unwrap_or(trimmed.len());
        signature.push(' ');
        signature.push_str(&trimmed[..end]);
        if end < trimmed.len() {
            signatures.push(open.take().expect("a signature is being collected"));
        }
    }
    signatures
}

#[test]
fn a_pooled_connection_is_taken_in_exactly_one_module() {
    let mut owner_still_takes_connections = false;
    let mut sentinel_found = false;
    for (path, body) in sources() {
        if path == SENTINEL {
            sentinel_found = true;
            continue;
        }
        if path == POOL_OWNER {
            owner_still_takes_connections = body.contains("pool.get(");
            continue;
        }
        assert!(
            !body.contains("pool.get("),
            "{path} takes a connection out of the pool; only {POOL_OWNER} may do that, \
             because a connection held anywhere else outlives a network await and \
             starves every other request of the few connections the pool may open"
        );
    }
    assert!(
        sentinel_found,
        "{SENTINEL} did not find itself; the paths this guard compares against are stale"
    );
    assert!(
        owner_still_takes_connections,
        "{POOL_OWNER} no longer takes connections; point this guard at the module that does \
         instead of leaving it green over nothing"
    );
}

#[test]
fn the_module_that_holds_a_connection_depends_on_nothing_that_talks_to_the_network() {
    let body = read(POOL_OWNER);
    let roots = use_roots(&body);
    assert!(
        !roots.is_empty(),
        "{POOL_OWNER} imports nothing at all; this guard is reading the wrong file"
    );
    for root in roots {
        assert!(
            ALLOWED_USE_ROOTS.contains(&root.as_str()),
            "{POOL_OWNER} now depends on `{root}`; a connection is held across every await \
             in this module, so anything that can wait on the network belongs outside it"
        );
    }
    for token in NETWORK_TOKENS {
        assert!(
            !body.contains(token),
            "{POOL_OWNER} mentions `{token}`; a pooled connection must never be held \
             across a call that waits on SoundCloud, storage or another service"
        );
    }
}

#[test]
fn a_pooled_connection_never_leaves_the_module_that_took_it() {
    let body = read(POOL_OWNER);
    let signatures = public_signatures(&body);
    assert!(
        !signatures.is_empty(),
        "{POOL_OWNER} exposes no public functions; this guard is reading the wrong file"
    );
    for signature in signatures {
        let Some(returns) = signature.split("->").nth(1) else {
            continue;
        };
        for escaping in ESCAPING_TYPES {
            assert!(
                !returns.contains(escaping),
                "{POOL_OWNER} hands a `{escaping}` to its callers in `{}`; \
                 the caller would then hold a connection across its own network work",
                signature.trim()
            );
        }
    }
}
