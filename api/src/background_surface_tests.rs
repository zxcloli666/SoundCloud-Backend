use std::fs;
use std::path::{Path, PathBuf};

const DETACHED: &[&str] = &[
    "tokio::spawn",
    "spawn_blocking",
    "tokio::time::interval",
    "JoinSet::new",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn balanced_slice(source: &str, open_at: usize) -> &str {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    for (offset, byte) in bytes[open_at..].iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open_at..=open_at + offset];
                }
            }
            _ => {}
        }
    }
    &source[open_at..]
}

fn without_test_items(source: &str) -> String {
    let mut kept = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("#[cfg(test)]") {
        let at = cursor + found;
        kept.push_str(&source[cursor..at]);
        let rest = &source[at..];
        let consumed = match (rest.find('{'), rest.find(';')) {
            (Some(brace), semicolon) if semicolon.is_none_or(|end| brace < end) => {
                brace + balanced_slice(rest, brace).len()
            }
            (_, Some(semicolon)) => semicolon + 1,
            _ => rest.len(),
        };
        cursor = at + consumed;
    }
    kept.push_str(&source[cursor..]);
    kept
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
                found.push((shown(&path), without_test_items(&body)));
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

#[test]
fn the_serving_process_starts_no_work_of_its_own() {
    let mut detached: Vec<String> = Vec::new();

    for (file, body) in serving_sources() {
        for (number, line) in body.lines().enumerate() {
            for pattern in DETACHED {
                if line.contains(pattern) {
                    detached.push(format!("{file}:{}: {}", number + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        detached.is_empty(),
        "api answers requests; every cron, reaper, consumer and fire-and-forget task belongs \
         to jobs, where it has a lease, a retry budget and somewhere to report failure. A task \
         spawned here dies with the request and nobody learns it never ran:\n  {}",
        detached.join("\n  ")
    );
}

#[test]
fn the_scan_would_notice_a_task_spawned_next_to_a_handler() {
    let planted = "fn serve() {\n    tokio::spawn(async {});\n}\n\n#[cfg(test)]\nmod tests {\n    fn probe() { tokio::spawn(async {}); }\n}\n";
    let serving = without_test_items(planted);

    assert!(
        serving.contains("tokio::spawn"),
        "a spawn in serving code must survive the test-stripping"
    );
    assert_eq!(
        serving.matches("tokio::spawn").count(),
        1,
        "and the one inside `#[cfg(test)]` must not, or every test harness would read as a \
         violation"
    );
}

#[test]
fn the_scan_reads_the_whole_serving_tree() {
    let sources = serving_sources();
    assert!(
        sources.len() >= 200,
        "only {} serving files were read; the crate is larger than that",
        sources.len()
    );
    assert!(
        sources.iter().any(|(file, _)| file.ends_with("main.rs")),
        "main.rs is where a stray task would most naturally be started"
    );
}
