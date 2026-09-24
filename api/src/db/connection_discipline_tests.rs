use std::fs;
use std::path::{Path, PathBuf};

const NETWORK_CALLS: [&str; 9] = [
    "self.sc.",
    "self.read.",
    "api_get_value",
    "self.cache.",
    "self.qdrant.",
    "enqueue_opportunistic",
    "with_access_token",
    "tokens.chain",
    "ctx.access_token(",
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
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("test"))
                && let Ok(body) = fs::read_to_string(&path)
            {
                out.push((path, body));
            }
        }
    }
    out
}

fn opens_a_connection(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("let mut ")
        && (line.contains(".begin().await") || line.contains(".acquire().await"))
}

fn closes_a_connection(line: &str) -> bool {
    line.contains(".commit().await") || line.contains(".rollback().await")
}

fn connection_blocks(body: &str) -> usize {
    body.lines().filter(|line| opens_a_connection(line)).count()
}

fn held_blocks(body: &str) -> Vec<(usize, Vec<(usize, String)>)> {
    let lines: Vec<&str> = body.lines().collect();
    let mut blocks = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        if !opens_a_connection(line) {
            continue;
        }
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, candidate)| closes_a_connection(candidate))
            .map(|(index, _)| index)
            .unwrap_or_else(|| lines.len().min(start + 80));
        let offenders: Vec<(usize, String)> = lines[start..=end.min(lines.len() - 1)]
            .iter()
            .enumerate()
            .filter(|(_, candidate)| NETWORK_CALLS.iter().any(|call| candidate.contains(call)))
            .map(|(offset, candidate)| (start + offset + 1, (*candidate).trim().to_owned()))
            .collect();
        if !offenders.is_empty() {
            blocks.push((start + 1, offenders));
        }
    }
    blocks
}

#[test]
fn no_pooled_connection_is_held_across_a_network_await() {
    let mut reported = Vec::new();
    let mut inspected = 0;
    for (path, body) in sources() {
        inspected += connection_blocks(&body);
        for (line, offenders) in held_blocks(&body) {
            for (offender_line, text) in offenders {
                reported.push(format!(
                    "{}:{} holds a connection opened at line {line} across `{text}`",
                    path.display(),
                    offender_line
                ));
            }
        }
    }

    assert!(
        inspected >= 20,
        "the guard found only {inspected} connection blocks, so it is no longer reading the \
         sources it is supposed to police"
    );
    assert!(
        reported.is_empty(),
        "a pooled PostgreSQL connection must never stay open across SoundCloud, Redis, Qdrant or \
         queue work; release it before the call and take a new one after: {reported:#?}"
    );
}

#[test]
fn the_guard_sees_a_connection_held_across_a_network_call() {
    let offending = "    let mut tx = self.pg.begin().await?;\n\
                     let profile = self.read.user(id).await?;\n\
                     tx.commit().await?;\n";
    assert_eq!(held_blocks(offending).len(), 1);

    let compliant = "    let profile = self.read.user(id).await?;\n\
                     let mut tx = self.pg.begin().await?;\n\
                     tx.commit().await?;\n";
    assert!(held_blocks(compliant).is_empty());
}
