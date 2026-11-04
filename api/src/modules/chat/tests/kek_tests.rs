use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use uuid::Uuid;

use super::KekRing;
use crate::modules::chat::ChatError;

struct KeysDir(PathBuf);

impl KeysDir {
    fn new(files: &[(&str, Vec<u8>)]) -> Self {
        let dir = std::env::temp_dir().join(format!("scd-chat-keks-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("a keys dir");
        for (name, bytes) in files {
            fs::write(dir.join(name), bytes).expect("a key file");
        }
        Self(dir)
    }
}

impl Drop for KeysDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ck_vector() -> Value {
    serde_json::from_str(sc_chat_wire::vectors::CK).expect("vector json")
}

fn hex32(vector: &Value, field: &str) -> [u8; 32] {
    hex::decode(vector[field].as_str().expect("a hex field"))
        .expect("hex")
        .try_into()
        .expect("32 bytes")
}

fn ring() -> KekRing {
    KekRing::new(
        2,
        1,
        BTreeMap::from([(1, [1; 32]), (2, [2; 32])]),
        BTreeMap::from([(1, [3; 32])]),
    )
    .expect("a ring")
}

#[test]
fn the_ring_loads_every_kek_file_and_skips_the_rest() {
    let dir = KeysDir::new(&[
        ("kek-1", vec![1; 32]),
        ("kek-2", vec![2; 32]),
        ("reports-kek-1", vec![3; 32]),
        ("ock-1", vec![4; 32]),
        ("kek-notes.txt", b"old".to_vec()),
    ]);
    let ring = KekRing::load(&dir.0, 2, 1).expect("a ring");
    assert_eq!((ring.current(), ring.reports_current()), (2, 1));

    let conversation = Uuid::new_v4();
    let wrapped = ring.wrap(conversation, 0, &[9; 32]).expect("a wrap");
    let older = KekRing::load(&dir.0, 1, 1).expect("the older ring");
    assert_eq!(older.unwrap(2, conversation, 0, &wrapped), Ok([9; 32]));
}

#[test]
fn a_missing_current_key_stops_the_boot() {
    let only_old = KeysDir::new(&[("kek-1", vec![1; 32]), ("reports-kek-1", vec![3; 32])]);
    assert!(KekRing::load(&only_old.0, 2, 1).is_err());
    assert!(KekRing::load(&only_old.0, 1, 2).is_err());

    let short = KeysDir::new(&[("kek-1", vec![1; 31]), ("reports-kek-1", vec![3; 32])]);
    let refused = KekRing::load(&short.0, 1, 1)
        .err()
        .expect("a short key fails");
    assert!(refused.to_string().contains("kek-1"));

    let absent = std::env::temp_dir().join(format!("scd-chat-none-{}", Uuid::new_v4()));
    assert!(KekRing::load(&absent, 1, 1).is_err());
}

#[test]
fn a_stored_key_from_the_chat_wire_vector_unwraps() {
    let vector = ck_vector();
    let ring = KekRing::new(
        1,
        1,
        BTreeMap::from([(1, hex32(&vector, "kek"))]),
        BTreeMap::from([(1, [3; 32])]),
    )
    .expect("a ring");
    let conversation =
        Uuid::parse_str(vector["conversation_id"].as_str().expect("an id")).expect("a uuid");
    let wrapped = hex::decode(vector["wrapped"].as_str().expect("hex")).expect("hex");
    assert_eq!(
        ring.unwrap(1, conversation, 3, &wrapped),
        Ok(hex32(&vector, "ck"))
    );
}

#[test]
fn a_wrapped_key_is_bound_to_its_conversation_epoch_and_kek() {
    let ring = ring();
    let conversation = Uuid::new_v4();
    let ck = [7; 32];
    let wrapped = ring.wrap(conversation, 4, &ck).expect("a wrap");
    assert_ne!(
        wrapped,
        ring.wrap(conversation, 4, &ck).expect("a wrap"),
        "every wrap takes a fresh nonce"
    );
    assert_eq!(ring.unwrap(2, conversation, 4, &wrapped), Ok(ck));
    assert_eq!(
        ring.unwrap(2, conversation, 5, &wrapped),
        Err(ChatError::Internal)
    );
    assert_eq!(
        ring.unwrap(2, Uuid::new_v4(), 4, &wrapped),
        Err(ChatError::Internal)
    );
    assert_eq!(
        ring.unwrap(1, conversation, 4, &wrapped),
        Err(ChatError::Internal)
    );
    assert_eq!(
        ring.unwrap(9, conversation, 4, &wrapped),
        Err(ChatError::Internal)
    );
    assert_eq!(ring.wrap(conversation, -1, &ck), Err(ChatError::Internal));
}

#[test]
fn evidence_opens_only_under_its_reports_kek() {
    let ring = ring();
    let sealed = ring.seal_evidence(b"reported range").expect("sealed");
    assert_eq!(
        ring.open_evidence(1, &sealed),
        Ok(b"reported range".to_vec())
    );
    assert_eq!(ring.open_evidence(2, &sealed), Err(ChatError::Internal));
    let mut tampered = sealed.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert_eq!(ring.open_evidence(1, &tampered), Err(ChatError::Internal));

    let next = KekRing::new(
        2,
        2,
        BTreeMap::from([(2, [2; 32])]),
        BTreeMap::from([(1, [3; 32]), (2, [5; 32])]),
    )
    .expect("a rotated ring");
    assert_eq!(
        next.open_evidence(1, &sealed),
        Ok(b"reported range".to_vec()),
        "old evidence stays readable after a reports kek rotation"
    );
}
