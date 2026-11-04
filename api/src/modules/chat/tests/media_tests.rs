use std::collections::HashMap;
use std::fs;

use super::MediaConfig;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

const PROD: [(&str, &str); 9] = [
    ("CHAT_MEDIA_UPLOADS", "false"),
    ("CHAT_STORES", "1"),
    ("CHAT_STORE_WRITE", "1"),
    ("CHAT_STORE_1_ENDPOINT", "https://s3.int.scnative.dev:8443"),
    ("CHAT_STORE_1_PRESIGN", "https://s3.scnative.dev"),
    ("CHAT_STORE_1_BUCKET", "scd-chat"),
    ("CHAT_STORE_1_ACCESS_KEY", "AKIA"),
    ("CHAT_STORE_1_CAP_GB", "200"),
    ("UNRELATED", "x"),
];

#[test]
fn the_phase_two_names_of_the_chat_env_parse_into_one_store() {
    let dir = tempfile_dir("media-parse");
    fs::write(dir.join("s3-secret-1"), "top-secret\n").expect("a secret file");
    let media = MediaConfig::parse(env(&PROD), &dir);
    assert!(!media.uploads);
    assert_eq!(media.write, Some(1));
    assert_eq!(media.stores.len(), 1);
    let store = &media.stores[0];
    assert_eq!(store.id, 1);
    assert_eq!(store.bucket, "scd-chat");
    assert_eq!(store.cap_gb, 200);
    assert_eq!(
        store.secret.as_ref().map(|s| s.expose().as_str()),
        Some("top-secret")
    );
    assert!(!format!("{store:?}").contains("top-secret"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_missing_secret_file_leaves_the_store_without_a_secret() {
    let dir = tempfile_dir("media-nosecret");
    let media = MediaConfig::parse(env(&PROD), &dir);
    assert_eq!(media.stores.len(), 1);
    assert!(media.stores[0].secret.is_none());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn nothing_set_is_off_and_empty() {
    let media = MediaConfig::parse(env(&[]), std::path::Path::new("/nonexistent"));
    assert!(!media.uploads && media.stores.is_empty() && media.write.is_none());
}

#[test]
fn broken_entries_are_skipped_and_never_stop_the_boot() {
    let media = MediaConfig::parse(
        env(&[
            ("CHAT_MEDIA_UPLOADS", "true"),
            ("CHAT_STORES", "x, 2 ,,3"),
            ("CHAT_STORE_WRITE", "many"),
            ("CHAT_STORE_2_BUCKET", "only-a-bucket"),
        ]),
        std::path::Path::new("/nonexistent"),
    );
    assert!(media.uploads);
    assert!(media.stores.is_empty());
    assert_eq!(media.write, None);
}

fn tempfile_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("chat-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temp dir");
    dir
}
