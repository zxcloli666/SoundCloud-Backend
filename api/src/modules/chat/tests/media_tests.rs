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
    let media = MediaConfig::parse(env(&PROD), &dir).expect("a media config");
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
    let media = MediaConfig::parse(env(&PROD), &dir).expect("a media config");
    assert_eq!(media.stores.len(), 1);
    assert!(media.stores[0].secret.is_none());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn nothing_set_is_off_and_empty() {
    let media =
        MediaConfig::parse(env(&[]), std::path::Path::new("/nonexistent")).expect("a media config");
    assert!(!media.uploads && media.stores.is_empty() && media.write.is_none());
}

#[test]
fn broken_entries_are_skipped_while_uploads_are_off() {
    let media = MediaConfig::parse(
        env(&[
            ("CHAT_MEDIA_UPLOADS", "false"),
            ("CHAT_STORES", "x, 2 ,,3"),
            ("CHAT_STORE_WRITE", "many"),
            ("CHAT_STORE_2_BUCKET", "only-a-bucket"),
        ]),
        std::path::Path::new("/nonexistent"),
    )
    .expect("uploads are off");
    assert!(!media.uploads);
    assert!(media.stores.is_empty());
    assert_eq!(media.write, None);
}

fn with_uploads(
    changes: &[(&'static str, Option<&'static str>)],
) -> Vec<(&'static str, &'static str)> {
    let mut pairs: Vec<(&str, &str)> = PROD
        .iter()
        .map(|&(name, value)| match name {
            "CHAT_MEDIA_UPLOADS" => (name, "true"),
            _ => (name, value),
        })
        .collect();
    for &(name, value) in changes {
        pairs.retain(|(kept, _)| *kept != name);
        if let Some(value) = value {
            pairs.push((name, value));
        }
    }
    pairs
}

fn refusal(changes: &[(&'static str, Option<&'static str>)], secret: bool) -> String {
    let dir = tempfile_dir(&format!("media-refuse-{}", uuid::Uuid::new_v4().simple()));
    if secret {
        fs::write(dir.join("s3-secret-1"), "top-secret").expect("a secret file");
        fs::write(dir.join("s3-secret-2"), "top-secret").expect("a secret file");
    }
    let parsed = MediaConfig::parse(env(&with_uploads(changes)), &dir);
    fs::remove_dir_all(dir).ok();
    match parsed {
        Ok(_) => String::new(),
        Err(error) => error.to_string(),
    }
}

#[test]
fn uploads_on_with_a_full_store_boots() {
    assert_eq!(refusal(&[], true), "");
}

#[test]
fn uploads_on_refuse_a_store_without_its_secret_file() {
    let error = refusal(&[], false);
    assert!(error.contains("store 1 has no secret"), "{error}");
    let dir = tempfile_dir("media-empty-secret");
    fs::write(dir.join("s3-secret-1"), " \n").expect("an empty secret file");
    let error = MediaConfig::parse(env(&with_uploads(&[])), &dir)
        .expect_err("an empty secret")
        .to_string();
    fs::remove_dir_all(dir).ok();
    assert!(error.contains("store 1 has no secret"), "{error}");
}

#[test]
fn uploads_on_refuse_a_missing_or_bad_cap() {
    for cap in [None, Some("lots"), Some("-5"), Some(" ")] {
        let error = refusal(&[("CHAT_STORE_1_CAP_GB", cap)], true);
        assert!(error.contains("CHAT_STORE_1_CAP_GB"), "{cap:?}: {error}");
    }
    assert_eq!(refusal(&[("CHAT_STORE_1_CAP_GB", Some("0"))], true), "");
}

#[test]
fn uploads_on_refuse_a_bad_or_missing_write_store() {
    let error = refusal(&[("CHAT_STORE_WRITE", Some("one"))], true);
    assert!(error.contains("CHAT_STORE_WRITE \"one\""), "{error}");
    let error = refusal(&[("CHAT_STORE_WRITE", None)], true);
    assert!(error.contains("CHAT_STORE_WRITE is not set"), "{error}");
}

#[test]
fn uploads_on_refuse_a_bad_id_and_an_incomplete_store() {
    let error = refusal(&[("CHAT_STORES", Some("1,x"))], true);
    assert!(error.contains("bad id \"x\""), "{error}");
    let error = refusal(&[("CHAT_STORES", Some("1,2"))], true);
    assert!(error.contains("store 2 needs"), "{error}");
}

#[test]
fn uploads_on_refuse_two_stores_on_one_bucket() {
    let second = [
        ("CHAT_STORES", Some("1,2")),
        (
            "CHAT_STORE_2_ENDPOINT",
            Some("https://s3.int.scnative.dev:8443"),
        ),
        ("CHAT_STORE_2_PRESIGN", Some("https://s3.scnative.dev")),
        ("CHAT_STORE_2_ACCESS_KEY", Some("AKIA")),
        ("CHAT_STORE_2_CAP_GB", Some("100")),
    ];
    let mut same = second.to_vec();
    same.push(("CHAT_STORE_2_BUCKET", Some("scd-chat")));
    let error = refusal(&same, true);
    assert!(
        error.contains("stores 1 and 2 share bucket \"scd-chat\""),
        "{error}"
    );
    let mut own = second.to_vec();
    own.push(("CHAT_STORE_2_BUCKET", Some("scd-chat-2")));
    assert_eq!(refusal(&own, true), "");
}

const SECOND_STORE: [(&str, Option<&str>); 6] = [
    ("CHAT_STORES", Some("1,2")),
    (
        "CHAT_STORE_2_ENDPOINT",
        Some("https://s3-2.int.scnative.dev:8443"),
    ),
    ("CHAT_STORE_2_PRESIGN", Some("https://s3.scnative.dev")),
    ("CHAT_STORE_2_ACCESS_KEY", Some("AKIA")),
    ("CHAT_STORE_2_CAP_GB", Some("100")),
    ("CHAT_STORE_2_BUCKET", Some("scd-chat-2")),
];

fn moving(
    uploads: bool,
    changes: &[(&'static str, Option<&'static str>)],
) -> anyhow::Result<MediaConfig> {
    let dir = tempfile_dir(&format!("media-move-{}", uuid::Uuid::new_v4().simple()));
    fs::write(dir.join("s3-secret-1"), "top-secret").expect("a secret file");
    fs::write(dir.join("s3-secret-2"), "top-secret").expect("a secret file");
    let mut all = SECOND_STORE.to_vec();
    all.extend_from_slice(changes);
    if !uploads {
        all.push(("CHAT_MEDIA_UPLOADS", Some("false")));
    }
    let parsed = MediaConfig::parse(env(&with_uploads(&all)), &dir);
    fs::remove_dir_all(dir).ok();
    parsed
}

#[test]
fn a_move_names_two_listed_stores_or_refuses_the_boot() {
    for (raw, pair) in [("1:2", (1, 2)), (" 2 : 1 ", (2, 1))] {
        let media = moving(true, &[("CHAT_MEDIA_MOVE", Some(raw))]).expect("a move");
        assert_eq!(media.moving, Some(pair), "{raw:?}");
    }
    assert_eq!(moving(true, &[]).expect("no move").moving, None);
    for raw in ["1:1", "1", "a:2", "1:2:3"] {
        let error = moving(true, &[("CHAT_MEDIA_MOVE", Some(raw))])
            .expect_err("a bad move")
            .to_string();
        assert!(error.contains("is not <from>:<to>"), "{raw:?}: {error}");
    }
    let error = moving(true, &[("CHAT_MEDIA_MOVE", Some("1:3"))])
        .expect_err("an unknown store")
        .to_string();
    assert!(
        error.contains("names a store CHAT_STORES does not list"),
        "{error}"
    );
}

#[test]
fn with_uploads_off_a_fault_keeps_the_mover_off() {
    let good = moving(false, &[("CHAT_MEDIA_MOVE", Some("1:2"))]).expect("uploads are off");
    assert_eq!(good.moving, Some((1, 2)));
    for faulty in [
        [("CHAT_MEDIA_MOVE", Some("1:3")), ("UNRELATED", None)],
        [
            ("CHAT_MEDIA_MOVE", Some("1:2")),
            ("CHAT_STORE_2_BUCKET", Some("scd-chat")),
        ],
        [
            ("CHAT_MEDIA_MOVE", Some("1:2")),
            ("CHAT_STORE_1_CAP_GB", None),
        ],
    ] {
        let media = moving(false, &faulty).expect("uploads are off");
        assert_eq!(media.moving, None, "{faulty:?}");
    }
}

#[test]
fn every_fault_is_named_in_one_refusal() {
    let error = refusal(
        &[
            ("CHAT_STORE_1_CAP_GB", None),
            ("CHAT_STORE_WRITE", Some("w")),
        ],
        false,
    );
    for part in ["no secret", "CHAT_STORE_1_CAP_GB", "CHAT_STORE_WRITE \"w\""] {
        assert!(error.contains(part), "{part}: {error}");
    }
}

fn tempfile_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("chat-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temp dir");
    dir
}
