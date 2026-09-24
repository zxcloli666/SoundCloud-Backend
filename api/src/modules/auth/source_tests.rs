use std::path::{Path, PathBuf};

const THE_AUTH_PATH: [&str; 3] = [
    "src/common/session.rs",
    "src/modules/auth/service",
    "src/modules/auth/token_provider.rs",
];

const CACHE_TOKENS: [&str; 3] = ["redis", "Redis", "deadpool_redis"];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sources_under(relative: &str) -> Vec<(String, String)> {
    fn collect(path: &Path, found: &mut Vec<PathBuf>) {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).expect("the directory is readable") {
                collect(&entry.expect("the entry is readable").path(), found);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path.to_path_buf());
        }
    }
    let mut paths = Vec::new();
    collect(&crate_root().join(relative), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let shown = path
                .strip_prefix(crate_root())
                .expect("every source lives under the crate root")
                .to_string_lossy()
                .into_owned();
            let body = std::fs::read_to_string(&path).expect("every source is readable");
            (shown, body)
        })
        .collect()
}

#[test]
fn who_you_are_is_answered_by_postgres_and_by_nothing_else() {
    let mut read = 0;
    for relative in THE_AUTH_PATH {
        for (path, body) in sources_under(relative) {
            read += 1;
            for token in CACHE_TOKENS {
                assert!(
                    !body.contains(token),
                    "{path} reaches for `{token}`; the answer to who a request belongs to must \
                     come from PostgreSQL alone, or a cache outage becomes a sign-out for \
                     everyone at once"
                );
            }
        }
    }
    assert!(
        read >= 4,
        "only {read} files were read; this guard is pointed at the wrong place"
    );
}
