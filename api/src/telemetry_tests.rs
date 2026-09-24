use std::fs;
use std::path::{Path, PathBuf};

const NEVER_IN_A_LOG_LINE: [&str; 6] = [
    "access_token",
    "refresh_token",
    "secret_token",
    "client_secret",
    "claim_token",
    "session_id",
];

const LOGGING_MACROS: [&str; 8] = [
    "trace!(",
    "debug!(",
    "info!(",
    "warn!(",
    "error!(",
    "trace_span!(",
    "debug_span!(",
    "info_span!(",
];

fn sources() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_test = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains("test"));
            if is_test || path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            if let Ok(body) = fs::read_to_string(&path) {
                let body = match body.find("#[cfg(test)]") {
                    Some(at) => body[..at].to_owned(),
                    None => body,
                };
                out.push((path, body));
            }
        }
    }
    out
}

fn arguments_after(body: &str, at: usize) -> &str {
    let rest = &body[at..];
    let mut depth = 0usize;
    for (index, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..index];
                }
            }
            _ => {}
        }
    }
    rest
}

#[test]
fn no_credential_is_ever_handed_to_a_log_line() {
    let sources = sources();
    assert!(
        sources.len() >= 40,
        "the scan found almost nothing, so it proves nothing: {}",
        sources.len()
    );

    let mut leaks: Vec<String> = Vec::new();
    for (path, body) in &sources {
        for macro_name in LOGGING_MACROS {
            let mut from = 0usize;
            while let Some(found) = body[from..].find(macro_name) {
                let start = from + found + macro_name.len() - 1;
                let arguments = arguments_after(body, start);
                for secret in NEVER_IN_A_LOG_LINE {
                    if arguments.contains(secret) {
                        leaks.push(format!("{}: {macro_name} carries {secret}", path.display()));
                    }
                }
                from = start + 1;
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "a log line would carry a value that authenticates its bearer: {leaks:#?}"
    );
}

#[test]
fn the_scan_would_notice_a_leak_that_is_actually_there() {
    let planted = "warn!(session_id = %session_id, \"about to leak\");";

    let arguments = arguments_after(planted, planted.find("warn!(").expect("found") + 5);

    assert!(
        NEVER_IN_A_LOG_LINE
            .iter()
            .any(|secret| arguments.contains(secret)),
        "the extraction must see the arguments of a logging macro, saw {arguments:?}"
    );
}
