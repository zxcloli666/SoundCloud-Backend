use std::fs;
use std::path::{Path, PathBuf};

const NAMED_EXACTLY: &[&str] = &[
    "token",
    "secret",
    "password",
    "cookies",
    "authorization",
    "api_key",
    "signing_key",
    "ticket_key",
    "track_authorization",
    "relay_secret",
];

const NAMED_ENDING_IN: &[&str] = &["_token", "_secret", "_password", "_api_key"];

fn sources() -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut stack = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
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
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.ends_with(".rs") || name.contains("test") {
                continue;
            }
            if let Ok(body) = fs::read_to_string(&path) {
                sources.push((display(&path), body));
            }
        }
    }
    sources.sort();
    sources
}

fn display(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(path)
        .display()
        .to_string()
}

fn derives_debug(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("#[derive(") && line.contains("Debug")
}

fn declares_a_type(line: &str) -> bool {
    let line = line.trim_start();
    let line = line.strip_prefix("pub(crate) ").unwrap_or(line);
    let line = line.strip_prefix("pub ").unwrap_or(line);
    line.starts_with("struct ") || line.starts_with("enum ")
}

fn prints_itself_verbatim(declared: &str) -> bool {
    let declared = declared.trim().trim_end_matches(',').trim();
    let inner = declared
        .strip_prefix("Option<")
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(declared)
        .trim();
    matches!(
        inner,
        "String" | "&str" | "&'static str" | "Vec<u8>" | "Bytes" | "Box<str>"
    )
}

fn holds_a_secret(field: &str) -> Option<String> {
    let field = field.trim();
    if field.starts_with("//") || field.starts_with('#') || field.contains("Secret<") {
        return None;
    }
    let declaration = field
        .strip_prefix("pub(crate) ")
        .or_else(|| field.strip_prefix("pub "))
        .unwrap_or(field);
    let (name, declared) = declaration.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return None;
    }
    if !prints_itself_verbatim(declared) {
        return None;
    }
    let named =
        NAMED_EXACTLY.contains(&name) || NAMED_ENDING_IN.iter().any(|tail| name.ends_with(tail));
    named.then(|| name.to_owned())
}

#[test]
fn no_type_prints_itself_while_holding_a_capability() {
    let mut printable: Vec<String> = Vec::new();

    for (file, body) in sources() {
        let lines: Vec<&str> = body.lines().collect();
        let mut index = 0;
        while index < lines.len() {
            if !derives_debug(lines[index]) {
                index += 1;
                continue;
            }
            let mut head = index + 1;
            while head < lines.len() && lines[head].trim_start().starts_with('#') {
                head += 1;
            }
            if head >= lines.len() || !declares_a_type(lines[head]) {
                index += 1;
                continue;
            }
            let opened = lines[head];
            let mut depth = i32::from(opened.contains('{')) - i32::from(opened.contains('}'));
            let mut field = head + 1;
            while field < lines.len() && depth > 0 {
                if let Some(secret) = holds_a_secret(lines[field]) {
                    printable.push(format!(
                        "{file}:{}: {} derives Debug and holds `{secret}`",
                        field + 1,
                        opened.trim()
                    ));
                }
                depth += i32::from(lines[field].contains('{'));
                depth -= i32::from(lines[field].contains('}'));
                field += 1;
            }
            index = field;
        }
    }

    assert!(
        printable.is_empty(),
        "a derived Debug on a type holding a capability is one `tracing::debug!(?value)` away \
         from writing that capability to disk; wrap the field in `redact::Secret`, write the \
         Debug by hand, or drop the derive:\n{}",
        printable.join("\n")
    );
}

#[test]
fn the_scan_can_actually_see_a_type_that_would_print_a_capability() {
    let planted =
        "#[derive(Clone, Debug)]\npub struct Pretend {\n    pub access_token: String,\n}\n";
    let lines: Vec<&str> = planted.lines().collect();

    assert!(derives_debug(lines[0]));
    assert!(declares_a_type(lines[1]));
    assert_eq!(holds_a_secret(lines[2]).as_deref(), Some("access_token"));
    assert_eq!(
        holds_a_secret("    pub access_token: redact::Secret<String>,").as_deref(),
        None,
        "a wrapped field is the fix, not a finding"
    );
    assert_eq!(
        holds_a_secret("    pub ticket_key: StreamTicketKey,").as_deref(),
        None,
        "a type that already writes its own Debug is the fix, not a finding"
    );
    for ordinary in [
        "    pub track_id: String,",
        "    pub dedup_key: String,",
        "    pub idempotency_key: Uuid,",
        "    pub token_expires_at: DateTime<Utc>,",
    ] {
        assert_eq!(
            holds_a_secret(ordinary).as_deref(),
            None,
            "{ordinary} carries no capability; a guard that cries wolf gets silenced"
        );
    }
}
