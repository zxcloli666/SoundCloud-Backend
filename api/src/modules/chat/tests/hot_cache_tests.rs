use chrono::{Duration, Utc};
use uuid::Uuid;

use super::{HotCache, TAIL_BYTES, TAIL_ROWS};
use crate::modules::chat::model::MessageRow;

const CONV: Uuid = Uuid::from_u128(7);

fn row(seq: i64) -> MessageRow {
    MessageRow {
        conversation_id: CONV,
        seq,
        client_id: Uuid::from_u128(1_000 + u128::try_from(seq).expect("a positive seq")),
        sender_user: "101".to_owned(),
        sender_device: Some(Uuid::from_u128(5)),
        class: "post".to_owned(),
        epoch: 0,
        target_seq: None,
        body: Some(vec![1; 64]),
        created_at: Utc::now(),
        expires_at: None,
    }
}

fn seqs(rows: &[MessageRow]) -> Vec<i64> {
    rows.iter().map(|row| row.seq).collect()
}

fn cache_with(range: std::ops::RangeInclusive<i64>) -> HotCache {
    let cache = HotCache::new(1 << 30);
    for seq in range {
        cache.put(row(seq));
    }
    cache
}

#[test]
fn a_contiguous_tail_answers_everything_after_a_seq() {
    let cache = cache_with(1..=5);
    assert_eq!(seqs(&cache.after(CONV, 2).expect("covered")), vec![3, 4, 5]);
    assert_eq!(
        seqs(&cache.after(CONV, 0).expect("covered")),
        vec![1, 2, 3, 4, 5]
    );
    assert!(cache.after(CONV, 5).expect("covered").is_empty());
}

#[test]
fn a_seq_older_than_the_tail_falls_back_to_postgres() {
    let cache = cache_with(10..=12);
    assert!(cache.after(CONV, 8).is_none());
    assert!(cache.after(CONV, i64::MIN).is_none());
    assert!(cache.after(CONV, i64::MAX).is_none());
    assert_eq!(
        seqs(&cache.after(CONV, 9).expect("covered")),
        vec![10, 11, 12]
    );
}

#[test]
fn a_gap_from_an_out_of_order_put_is_never_served() {
    let cache = cache_with(1..=3);
    cache.put(row(5));
    assert!(cache.after(CONV, 2).is_none());
    cache.put(row(4));
    assert_eq!(seqs(&cache.after(CONV, 2).expect("covered")), vec![3, 4, 5]);
}

#[test]
fn an_expired_row_is_never_served() {
    let cache = cache_with(1..=2);
    let mut gone = row(3);
    gone.expires_at = Some(Utc::now() - Duration::seconds(1));
    cache.put(gone);
    let mut later = row(4);
    later.expires_at = Some(Utc::now() + Duration::hours(1));
    cache.put(later);
    assert_eq!(seqs(&cache.after(CONV, 1).expect("covered")), vec![2, 4]);
}

#[test]
fn a_tombstone_replaces_the_cached_row() {
    let cache = cache_with(1..=3);
    let mut tombstone = row(2);
    tombstone.body = None;
    cache.put(tombstone);
    let rows = cache.after(CONV, 0).expect("covered");
    assert_eq!(seqs(&rows), vec![1, 2, 3]);
    assert_eq!(rows[1].body, None);
}

#[test]
fn a_row_older_than_the_tail_does_not_open_a_hole() {
    let cache = cache_with(10..=11);
    cache.put(row(3));
    assert!(cache.after(CONV, 2).is_none());
    assert_eq!(seqs(&cache.after(CONV, 9).expect("covered")), vec![10, 11]);
}

#[test]
fn the_tail_keeps_only_the_newest_rows() {
    let last = i64::try_from(TAIL_ROWS).expect("small") + 10;
    let cache = cache_with(1..=last);
    assert!(cache.after(CONV, 9).is_none());
    assert_eq!(cache.after(CONV, 10).expect("covered").len(), TAIL_ROWS);
}

#[test]
fn the_tail_stays_under_its_byte_budget() {
    let cache = HotCache::new(1 << 30);
    for seq in 1..=40 {
        let mut big = row(seq);
        big.body = Some(vec![0; 48 * 1024]);
        cache.put(big);
    }
    let first_kept = (1..=40)
        .find(|seq| cache.after(CONV, seq - 1).is_some())
        .expect("some tail");
    let kept = 40 - first_kept + 1;
    assert!(usize::try_from(kept).expect("small") * 48 * 1024 <= TAIL_BYTES);
    assert!(kept > 1);
}

#[test]
fn forget_drops_the_conversation() {
    let cache = cache_with(1..=3);
    cache.forget(CONV);
    assert!(cache.after(CONV, 0).is_none());
}

#[test]
fn conversations_do_not_share_a_tail() {
    let cache = cache_with(1..=2);
    let other = Uuid::from_u128(8);
    assert!(cache.after(other, 0).is_none());
}
