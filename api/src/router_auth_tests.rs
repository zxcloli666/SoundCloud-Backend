use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const OPEN_ROUTES: &[&str] = &[
    "/health",
    "/auth/login",
    "/auth/login/status",
    "/auth/callback",
    "/auth/session",
    "/auth/logout",
    "/auth/link/create",
    "/auth/link/claim",
    "/auth/link/status",
    "/resolve",
];

fn modules_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn router_sources() -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut stack = vec![modules_dir()];
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
            if name.contains("test") {
                continue;
            }
            let Ok(body) = fs::read_to_string(&path) else {
                continue;
            };
            sources.push((path.display().to_string(), without_test_items(&body)));
        }
    }
    sources.sort();
    sources
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

fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
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

fn route_handlers(source: &str) -> Vec<(String, Vec<String>)> {
    let mut routes = Vec::new();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find(".route(") {
        let open_at = cursor + found + ".route".len();
        let call = balanced_slice(source, open_at);
        cursor = open_at + call.len();

        let Some(quote) = call.find('"') else {
            continue;
        };
        let rest = &call[quote + 1..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let path = rest[..end].to_owned();

        let mut handlers = Vec::new();
        for method in ["get(", "post(", "put(", "patch(", "delete(", "head("] {
            let mut inner = 0;
            while let Some(at) = call[inner..].find(method) {
                let start = inner + at + method.len();
                let reference: String = call[start..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                    .collect();
                if !reference.is_empty() {
                    handlers.push(reference);
                }
                inner = start;
            }
        }
        routes.push((path, handlers));
    }
    routes
}

fn signature_of<'a>(source: &'a str, handler: &str) -> Option<&'a str> {
    let needle = format!("fn {handler}(");
    let at = source.find(&needle)?;
    let open_at = at + needle.len() - 1;
    Some(balanced_slice(source, open_at))
}

fn guards(signature: &str) -> bool {
    signature.contains("AdminAuth")
        || signature.contains("SessionCtx")
        || signature.contains("OptionalSession")
        || signature.contains("RawSessionIdHeader")
}

fn resolve<'a>(sources: &'a [(String, String)], file: &str, reference: &str) -> Option<&'a str> {
    let name = reference.rsplit("::").next().unwrap_or_default();
    if let Some((_, body)) = sources.iter().find(|(path, _)| path == file)
        && let Some(signature) = signature_of(body, name)
    {
        return Some(signature);
    }
    let module = reference.strip_suffix(name)?.trim_end_matches("::");
    if module.is_empty() {
        return None;
    }
    let segment = module.rsplit("::").next().unwrap_or(module);
    sources
        .iter()
        .filter(|(path, _)| {
            path.contains(&format!("/{segment}/")) || path.ends_with(&format!("/{segment}.rs"))
        })
        .find_map(|(_, body)| signature_of(body, name))
}

#[test]
fn every_route_is_gated_or_explicitly_open() {
    let mut ungated: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unresolved: Vec<String> = Vec::new();

    let sources: Vec<(String, String)> = router_sources()
        .into_iter()
        .map(|(file, raw)| (file, strip_comments(&raw)))
        .collect();

    for (file, source) in &sources {
        if !source.contains(".route(") {
            continue;
        }
        for (path, handlers) in route_handlers(source) {
            if OPEN_ROUTES.contains(&path.as_str()) {
                continue;
            }
            if handlers.is_empty() {
                unresolved.push(format!("{file}: {path} has no recognizable handler"));
                continue;
            }
            for handler in handlers {
                match resolve(&sources, file, &handler) {
                    Some(signature) if guards(signature) => {}
                    Some(_) => ungated
                        .entry(path.clone())
                        .or_default()
                        .push(format!("{handler} ({file})")),
                    None => unresolved.push(format!("{file}: {path} -> fn {handler} not found")),
                }
            }
        }
    }

    assert!(
        unresolved.is_empty(),
        "route guard scan could not resolve some handlers, fix the scan before trusting it:\n{}",
        unresolved.join("\n")
    );
    assert!(
        ungated.is_empty(),
        "these routes reach a handler without AdminAuth, SessionCtx or OptionalSession, \
         and premium_gate is a passthrough whenever premium_reserve is off:\n{}",
        ungated
            .iter()
            .map(|(path, handlers)| format!("  {path} -> {}", handlers.join(", ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_open_list_only_holds_routes_that_exist() {
    let mut known: Vec<String> = Vec::new();
    for (_, raw) in router_sources() {
        let source = strip_comments(&raw);
        if !source.contains(".route(") {
            continue;
        }
        for (path, _) in route_handlers(&source) {
            known.push(path);
        }
    }
    let missing: Vec<&&str> = OPEN_ROUTES
        .iter()
        .filter(|path| !known.contains(&(**path).to_owned()))
        .collect();
    assert!(
        missing.is_empty(),
        "the open-route allow list names routes that no longer exist: {missing:?}"
    );
}

fn first_parameter(signature: &str) -> String {
    let inner = signature
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(signature);
    let mut depth = 0_i32;
    let mut first = String::new();
    for character in inner.chars() {
        match character {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => break,
            _ => {}
        }
        first.push(character);
    }
    first.trim().to_owned()
}

#[test]
fn an_admin_route_is_gated_by_admin_auth_and_by_nothing_weaker() {
    let sources: Vec<(String, String)> = router_sources()
        .into_iter()
        .map(|(file, raw)| (file, strip_comments(&raw)))
        .collect();

    let mut checked = 0;
    let mut wrong: Vec<String> = Vec::new();
    for (file, source) in &sources {
        if !source.contains(".route(") {
            continue;
        }
        for (path, handlers) in route_handlers(source) {
            if !path.starts_with("/admin") {
                continue;
            }
            for handler in handlers {
                let Some(signature) = resolve(&sources, file, &handler) else {
                    wrong.push(format!("{file}: {path} -> fn {handler} not found"));
                    continue;
                };
                checked += 1;
                if !signature.contains("AdminAuth") {
                    wrong.push(format!(
                        "{path} -> {handler} ({file}): a session is not an admin token"
                    ));
                    continue;
                }
                if !first_parameter(signature).contains("AdminAuth") {
                    wrong.push(format!(
                        "{path} -> {handler} ({file}): AdminAuth is not the first argument, so \
                         an earlier extractor can answer the request before the token is checked"
                    ));
                }
            }
        }
    }

    assert!(
        checked >= 60,
        "only {checked} admin handlers were examined; the tree holds more than that, so this \
         guard is reading a truncated view of it and the handlers it never reached are unchecked"
    );
    assert!(
        wrong.is_empty(),
        "every /admin route must be closed by AdminAuth before anything else runs:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn no_module_can_hide_its_routes_from_the_guard_scan() {
    let mut hidden: Vec<String> = Vec::new();
    for (file, scanned) in router_sources() {
        let Ok(raw) = fs::read_to_string(&file) else {
            continue;
        };
        let declared = route_handlers(&strip_comments(&raw)).len();
        if declared == 0 {
            continue;
        }
        let seen = route_handlers(&strip_comments(&scanned)).len();
        if seen < declared {
            hidden.push(format!(
                "{file}: the scan sees {seen} of {declared} routes, so the rest are gated by \
                 nothing this test can read"
            ));
        }
    }
    assert!(
        hidden.is_empty(),
        "a test module declared above a router makes every route below it invisible to the \
         guard scan, and an ungated admin handler then passes unnoticed:\n{}",
        hidden.join("\n")
    );
}

#[test]
fn a_test_module_declared_before_the_router_keeps_the_router_visible() {
    let source = "use crate::x;\n\n#[cfg(test)]\n#[path = \"paging_tests.rs\"]\nmod paging_tests;\n\npub fn router() -> Router {\n    Router::new().route(\"/admin/a\", get(a))\n}\n\n#[cfg(test)]\nmod tests {\n    fn helper() -> Router {\n        Router::new().route(\"/admin/b\", get(b))\n    }\n}\n";
    let kept = without_test_items(source);
    assert_eq!(
        route_handlers(&kept),
        vec![("/admin/a".to_owned(), vec!["a".to_owned()])],
        "a `#[cfg(test)] mod x;` declaration must cost exactly its own line, while a \
         `#[cfg(test)] mod tests {{ … }}` block must cost exactly its own braces"
    );
}

#[test]
fn the_first_parameter_is_read_as_written() {
    assert_eq!(
        first_parameter("(_: AdminAuth, State(st): State<AppState>)"),
        "_: AdminAuth"
    );
    assert_eq!(
        first_parameter("(State(st): State<AppState>, _: AdminAuth)"),
        "State(st): State<AppState>"
    );
    assert_eq!(
        first_parameter("(\n    State(st): State<AppState>,\n    _: AdminAuth,\n)"),
        "State(st): State<AppState>"
    );
}
