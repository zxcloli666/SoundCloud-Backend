use std::fs;
use std::path::{Path, PathBuf};

const REACHES_SOUNDCLOUD: [&str; 10] = [
    "ScReadService",
    "ScClient",
    "api_get_value",
    "try_with_chain",
    "with_access_token",
    "ctx.access_token(",
    "st.resolve.",
    "st.sc.",
    "self.read.",
    "self.sc.",
];

const ALLOWED: [(&str, &str); 5] = [
    (
        "auth",
        "AUTH_CONTROL: login, token exchange, explicit refresh",
    ),
    (
        "tracks",
        "USER_PRIVATE: secret_token detail and the stream token readiness check",
    ),
    ("playlists", "USER_PRIVATE: secret_token detail"),
    ("resolve", "RESOLVE_MISS: an unknown permalink or URN"),
    (
        "admin",
        "RESOLVE_MISS: an operator resolves a link by hand behind AdminAuth",
    ),
];

fn serving_sources() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("modules"),
    ];
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

fn module_of(path: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("modules");
    path.strip_prefix(&root)
        .ok()
        .and_then(|rest| rest.components().next())
        .map(|first| {
            first
                .as_os_str()
                .to_string_lossy()
                .trim_end_matches(".rs")
                .to_owned()
        })
        .unwrap_or_default()
}

#[test]
fn only_the_documented_families_can_reach_soundcloud_from_a_request() {
    let sources = serving_sources();
    assert!(
        sources.len() >= 40,
        "the scan found almost nothing, so it proves nothing: {}",
        sources.len()
    );

    let mut unexpected: Vec<String> = Vec::new();
    for (path, body) in &sources {
        let module = module_of(path);
        if ALLOWED.iter().any(|(name, _)| *name == module) {
            continue;
        }
        for marker in REACHES_SOUNDCLOUD {
            if body.contains(marker) {
                unexpected.push(format!("{}: {marker}", path.display()));
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "a serving family outside the allowed classes reaches SoundCloud in a request: {unexpected:#?}"
    );
}

const CLASSIFIED_ROUTES: [&str; 63] = [
    "/admin/albums",
    "/admin/artists",
    "/admin/artists/{artist_id}",
    "/admin/artists/{artist_id}/sc-accounts/{sc_user_id}/detach-tracks",
    "/admin/artists/{artist_id}/sc-accounts/{sc_user_id}/tracks",
    "/admin/artists/{artist_id}/tracks",
    "/admin/auth/overview",
    "/admin/catalog/weak-credits",
    "/admin/catalog/weak-credits/accept",
    "/admin/catalog/weak-credits/reject",
    "/admin/http-stats",
    "/admin/hydrate",
    "/admin/index-usage",
    "/admin/infra",
    "/admin/maintenance/mb-artist-names",
    "/admin/maintenance/renormalize",
    "/admin/maintenance/status",
    "/admin/metrics",
    "/admin/oauth-apps/health",
    "/admin/playlists/legacy",
    "/admin/playlists/legacy/{archive_id}/abandon",
    "/admin/resolve",
    "/admin/slow-queries",
    "/admin/stats",
    "/admin/sync-queue",
    "/admin/sync-queue/flush",
    "/admin/sync-queue/items",
    "/admin/sync-queue/purge",
    "/admin/tracks",
    "/admin/tracks/{track_id}",
    "/admin/tracks/{track_id}/album",
    "/admin/tracks/{track_id}/blocks/{artist_id}",
    "/admin/tracks/{track_id}/credits",
    "/admin/tracks/{track_id}/credits/{artist_id}",
    "/admin/tracks/{track_id}/detach-artist",
    "/admin/tracks/{track_id}/primary-artist",
    "/admin/wanted-tracks",
    "/admin/wanted-tracks/{id}/link",
    "/admin/wanted-tracks/{id}/status",
    "/auth/callback",
    "/auth/link/claim",
    "/auth/link/create",
    "/auth/link/status",
    "/auth/login",
    "/auth/login/status",
    "/auth/logout",
    "/auth/session",
    "/auth/soundcloud",
    "/auth/soundcloud/refresh",
    "/playlists",
    "/playlists/{playlist_urn}",
    "/playlists/{playlist_urn}/reposters",
    "/playlists/{playlist_urn}/sharing",
    "/playlists/{playlist_urn}/tracks",
    "/resolve",
    "/tracks",
    "/tracks/{track_urn}",
    "/tracks/{track_urn}/comments",
    "/tracks/{track_urn}/favoriters",
    "/tracks/{track_urn}/related",
    "/tracks/{track_urn}/reposters",
    "/tracks/{track_urn}/sharing",
    "/tracks/{track_urn}/stream",
];

fn routes_of_allowed_families() -> Vec<String> {
    let mut routes: Vec<String> = Vec::new();
    for (path, body) in serving_sources() {
        let module = module_of(&path);
        if !ALLOWED.iter().any(|(name, _)| *name == module) {
            continue;
        }
        let mut rest = body.as_str();
        while let Some(at) = rest.find(".route(") {
            rest = &rest[at + ".route(".len()..];
            let after_space = rest.trim_start();
            let Some(quoted) = after_space.strip_prefix('"') else {
                continue;
            };
            let Some(end) = quoted.find('"') else { break };
            let route = quoted[..end].to_owned();
            if !routes.contains(&route) {
                routes.push(route);
            }
            rest = &quoted[end..];
        }
    }
    routes.sort();
    routes
}

#[test]
fn every_route_of_a_family_that_may_reach_soundcloud_is_classified() {
    let found = routes_of_allowed_families();
    let mut expected: Vec<String> = CLASSIFIED_ROUTES.iter().map(|r| (*r).to_owned()).collect();
    expected.sort();

    let added: Vec<&String> = found.iter().filter(|r| !expected.contains(r)).collect();
    let gone: Vec<&String> = expected.iter().filter(|r| !found.contains(r)).collect();

    assert!(
        added.is_empty(),
        "a new route lives in a family that may reach SoundCloud and is not classified in \
         docs/endpoint-authority-matrix.md: {added:#?}"
    );
    assert!(
        gone.is_empty(),
        "these routes are classified but no longer exist; drop them from the matrix: {gone:#?}"
    );
}

#[test]
fn every_allowed_family_still_exists_and_still_needs_the_exception() {
    let sources = serving_sources();
    for (module, reason) in ALLOWED {
        let touches = sources.iter().any(|(path, body)| {
            module_of(path) == module && REACHES_SOUNDCLOUD.iter().any(|m| body.contains(m))
        });
        assert!(
            touches,
            "{module} no longer reaches SoundCloud ({reason}); drop it from the allowed list"
        );
    }
}
