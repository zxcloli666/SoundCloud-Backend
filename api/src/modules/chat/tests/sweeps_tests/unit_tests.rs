use std::collections::{BTreeMap, HashMap, HashSet};

use uuid::Uuid;

use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::expired::by_conversation;
use crate::modules::chat::sweeps::hooks::gone_member;
use crate::modules::chat::sweeps::marks::{conversation_of, digest, read_marks_prefix, seqs};
use crate::modules::chat::sweeps::schedule::Sweep;
use crate::modules::chat::test_support::offline_chat_state;

#[test]
fn expired_rows_are_grouped_per_conversation_in_seq_order() {
    let first = Uuid::from_u128(1);
    let second = Uuid::from_u128(2);
    let grouped = by_conversation(vec![(second, 9), (first, 4), (first, 2), (second, 9)]);
    assert_eq!(
        grouped,
        BTreeMap::from([(first, vec![2, 4]), (second, vec![9])])
    );
}

#[test]
fn gone_entries_name_the_conversation_and_its_seqs() {
    let conversation = Uuid::from_u128(7);
    assert_eq!(
        gone_member(conversation, &[3, 4, 10]),
        format!("gone:{conversation}:3,4,10")
    );
}

#[test]
fn the_marks_scan_finds_every_read_marks_key_and_nothing_else() {
    let prefix = read_marks_prefix();
    let conversation = Uuid::now_v7();
    assert!(prefix.ends_with(':'), "{prefix}");
    assert_eq!(
        conversation_of(&prefix, &rt_keys::read_marks(conversation)),
        Some(conversation)
    );
    assert_eq!(
        conversation_of(&prefix, &rt_keys::delivered_marks(conversation)),
        None
    );
    assert_eq!(conversation_of(&prefix, &rt_keys::replay(b"enc")), None);
    assert_eq!(
        conversation_of(&prefix, &rt_keys::conversation_seq(conversation)),
        None
    );
}

#[test]
fn a_marks_digest_ignores_hash_order_and_sees_any_change() {
    let one = HashMap::from([("a".to_owned(), 3), ("b".to_owned(), 5)]);
    let same = HashMap::from([("b".to_owned(), 5), ("a".to_owned(), 3)]);
    let moved = HashMap::from([("a".to_owned(), 4), ("b".to_owned(), 5)]);
    assert_eq!(digest(&one), digest(&same));
    assert_ne!(digest(&one), digest(&moved));
}

#[test]
fn a_malformed_mark_is_skipped_instead_of_stalling_the_flush() {
    let raw = HashMap::from([
        ("a".to_owned(), "12".to_owned()),
        ("b".to_owned(), "x".to_owned()),
    ]);
    assert_eq!(seqs(raw), HashMap::from([("a".to_owned(), 12)]));
}

#[test]
fn every_sweep_has_its_own_name_and_a_sane_period() {
    let names: HashSet<&str> = Sweep::ALL.iter().map(|sweep| sweep.name()).collect();
    assert_eq!(names.len(), Sweep::ALL.len());
    for sweep in Sweep::ALL {
        assert!(sweep.every().as_secs() >= 30, "{sweep:?}");
        assert!(sweep.every().as_secs() <= 3_600, "{sweep:?}");
    }
    assert_eq!(Sweep::Marks.every().as_secs(), 30);
    assert_eq!(Sweep::Expired.every().as_secs(), 60);
}

#[test]
fn the_lease_outlives_two_missed_renewals() {
    assert!(
        crate::modules::chat::sweeps::leader::RENEW_EVERY * 2 < rt_keys::LEAD_TTL,
        "a single slow renewal must not hand the sweeps to another process"
    );
}

#[tokio::test]
async fn a_budget_runs_out_and_stops_at_shutdown() {
    let chat = offline_chat_state(true);
    let mut budget = Budget::new(2);
    assert!(budget.spend(&chat));
    assert!(budget.spend(&chat));
    assert!(!budget.spend(&chat));

    let mut budget = Budget::new(5);
    chat.shutdown.cancel();
    assert!(!budget.left(&chat));
    assert!(!budget.spend(&chat));
}
