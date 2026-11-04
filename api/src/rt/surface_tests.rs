use std::fs;
use std::path::{Path, PathBuf};

const REACHABLE_FROM_RT: [&str; 4] = [
    "src/rt",
    "src/modules/chat",
    "src/modules/rooms",
    "src/modules/auth/link_service.rs",
];

const NEVER_ON_RT: [&str; 11] = [
    "get_valid_access_token",
    "get_lasting_access_token",
    "refresh_soundcloud",
    "refresh_rejected_access_token",
    "refresh_connection",
    "mark_access_token_rejected",
    ".access_token(",
    ".lasting_access_token(",
    ".app_token(",
    ".admin_catalog(",
    "CoreRpc::new",
];

fn production_files(root: &Path, found: &mut Vec<PathBuf>) {
    if root.is_file() {
        found.push(root.to_owned());
        return;
    }
    for entry in fs::read_dir(root).expect("a readable source dir").flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() && name != "tests" {
            production_files(&path, found);
        } else if name.ends_with(".rs")
            && !name.ends_with("_tests.rs")
            && !name.contains("test_support")
            && name != "testing.rs"
        {
            found.push(path);
        }
    }
}

#[test]
fn no_rt_route_reaches_a_token_refresh_or_the_core_rpc() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for root in REACHABLE_FROM_RT {
        production_files(&manifest.join(root), &mut files);
    }
    assert!(files.len() > 100, "only {} files were scanned", files.len());
    let mut offenders = Vec::new();
    for file in &files {
        let body = fs::read_to_string(file).expect("a readable source file");
        for call in NEVER_ON_RT {
            if body.contains(call) {
                offenders.push(format!("{} calls {call}", file.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "chat-api never dials core, these reach a refresh or the core RPC: {offenders:?}"
    );
}

#[test]
fn rt_builds_its_auth_service_with_a_core_rpc_that_never_dials() {
    let state = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rt/state.rs"))
        .expect("the rt state");
    assert!(state.contains("CoreRpc::unreachable()"));
    assert!(!state.contains("NatsService"));
}
