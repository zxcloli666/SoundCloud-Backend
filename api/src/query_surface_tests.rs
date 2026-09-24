use std::fs;
use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files_under(root: PathBuf, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|name| name.to_str()) == Some(extension) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn every_line_of_rust() -> String {
    files_under(crate_root().join("src"), "rs")
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_query_file_is_reachable_from_the_code() {
    let root = crate_root().join("queries");
    let sources = every_line_of_rust();
    let orphans: Vec<String> = files_under(root.clone(), "sql")
        .into_iter()
        .filter_map(|path| {
            let named = path.strip_prefix(&root).ok()?.display().to_string();
            (!sources.contains(&named)).then_some(named)
        })
        .collect();

    assert!(
        orphans.is_empty(),
        "these queries are written, reviewed and migrated against, but nothing runs them; a \
         query that stopped being called is either a feature that quietly went missing or \
         dead weight that still has to compile:\n  {}",
        orphans.join("\n  ")
    );
}

#[test]
fn the_scan_reads_the_queries_it_claims_to_check() {
    let found = files_under(crate_root().join("queries"), "sql").len();
    assert!(
        found >= 300,
        "only {found} query files were read; this crate has more than that, so the guard is \
         looking at the wrong directory"
    );
}
