use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &["target", "worker", "worker-old", ".git", "node_modules"];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the api crate lives inside the workspace")
        .to_path_buf()
}

fn rust_sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![
        workspace_root().join("api/src"),
        workspace_root().join("jobs/src"),
        workspace_root().join("streaming/src"),
        workspace_root().join("backend-contracts/src"),
        workspace_root().join("utils"),
    ];
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
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn opens_a_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with("/*")
}

fn shown(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn the_code_carries_no_comments_because_the_names_carry_the_meaning() {
    let mut written: Vec<String> = Vec::new();

    for path in rust_sources() {
        let Ok(body) = fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in body.lines().enumerate() {
            if opens_a_comment(line) {
                written.push(format!("{}:{}: {}", shown(&path), number + 1, line.trim()));
            }
        }
    }

    assert!(
        written.is_empty(),
        "this project keeps its explanations in `docs/` and in conversation, never in the \
         source; a comment here is the one form of documentation nothing can keep honest:\n  {}",
        written.join("\n  ")
    );
}

#[test]
fn the_scan_reaches_every_crate_it_claims_to_read() {
    let sources = rust_sources();
    for crate_dir in ["api/src", "jobs/src", "streaming/src", "utils"] {
        let root = workspace_root().join(crate_dir);
        assert!(
            sources.iter().any(|path| path.starts_with(&root)),
            "{crate_dir} contributed no file, so this guard would pass however that crate \
             is written"
        );
    }
    assert!(
        sources.len() >= 400,
        "only {} files were read; the tree is larger than that",
        sources.len()
    );
}

#[test]
fn a_line_that_opens_a_comment_is_recognised_wherever_it_sits() {
    for line in ["// plain", "    /// doc", "//! module doc", "\t/* block */"] {
        assert!(opens_a_comment(line), "{line} opens a comment");
    }
    for line in [
        "let url = \"https://example.com\";",
        "    path.split(\"//\").next()",
        "",
    ] {
        assert!(!opens_a_comment(line), "{line} is code, not a comment");
    }
}
