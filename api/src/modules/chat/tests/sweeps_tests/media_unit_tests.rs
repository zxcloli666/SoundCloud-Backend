use chrono::{NaiveDate, TimeZone, Utc};

use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::sweeps::media::buckets_distinct;
use crate::modules::chat::sweeps::media::reconcile::{Cursor, Stage, ours, over};

fn at(day: u32, hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, hour, 30, 0)
        .single()
        .expect("a time")
}

#[test]
fn reconcile_starts_once_a_night_after_four_moscow_time() {
    let stores = [1, 2];
    assert_eq!(Cursor::due(None, &stores, at(5, 0)), None);
    let started = Cursor::due(None, &stores, at(5, 1)).expect("due");
    assert_eq!((started.stage, started.store), (Stage::List, 1));
    let mut finished = started.clone();
    finished.stage = Stage::Done;
    assert_eq!(
        Cursor::due(Some(finished.clone()), &stores, at(5, 23)),
        None
    );
    let next = Cursor::due(Some(finished), &stores, at(6, 1)).expect("the next night");
    assert_eq!(
        next.day,
        NaiveDate::from_ymd_opt(2026, 11, 6)
            .expect("a day")
            .to_string()
    );
    let mut halfway = started;
    halfway.after = "m/7".to_owned();
    assert_eq!(
        Cursor::due(Some(halfway.clone()), &stores, at(5, 0)),
        Some(halfway),
        "a run in progress resumes at any hour"
    );
}

#[test]
fn reconcile_walks_the_stores_in_order_then_recounts() {
    let stores = [1, 2];
    let mut cursor = Cursor::start("2026-11-05".to_owned(), &stores);
    assert_eq!((cursor.stage, cursor.store), (Stage::List, 1));
    cursor.after = "m/ff".to_owned();
    cursor.stage = Stage::Tail;
    cursor.next_store(&stores);
    assert_eq!(
        (cursor.stage, cursor.store, cursor.after.as_str()),
        (Stage::List, 2, "")
    );
    cursor.next_store(&stores);
    assert_eq!(cursor.stage, Stage::Recount);
    assert_eq!(Cursor::start("d".to_owned(), &[]).stage, Stage::Recount);
}

#[test]
fn only_keys_in_the_chat_key_shape_can_be_orphans() {
    assert!(ours("m/0123456789abcdef0123456789abcdef"));
    assert!(!ours("m/0123456789abcdef0123456789abcde"));
    assert!(!ours("soundcloud_tracks_1.m4a"));
    assert!(!ours("x/0123456789abcdef0123456789abcdef"));
    assert!(!ours("m/0123456789abcdef0123456789abcdeg"));
}

fn two_stores(second_bucket: &str) -> MediaConfig {
    MediaConfig::parse(
        |name| {
            let value = match name {
                "CHAT_STORES" => "1,2",
                "CHAT_STORE_WRITE" => "2",
                "CHAT_STORE_1_BUCKET" => "scd-chat",
                "CHAT_STORE_2_BUCKET" => second_bucket,
                _ if name.ends_with("_ENDPOINT") || name.ends_with("_PRESIGN") => "http://s3",
                _ if name.ends_with("_ACCESS_KEY") => "key",
                _ => return None,
            };
            Some(value.to_owned())
        },
        std::path::Path::new("/nonexistent"),
    )
    .expect("a media config")
}

#[test]
fn two_stores_on_one_bucket_keep_reconcile_off() {
    assert!(buckets_distinct(&two_stores("scd-chat-2")));
    assert!(!buckets_distinct(&two_stores("scd-chat")));
}

#[test]
fn a_night_stops_marking_or_deleting_past_its_cap() {
    assert!(!over(1_000, 0, 1_000));
    assert!(over(1_000, 1, 1_000));
    assert!(over(1_000, 1_000, 1));
    assert!(!over(1_000, 999, 0));
}

#[test]
fn a_cursor_on_a_store_that_left_the_config_moves_on() {
    let mut cursor = Cursor::start("2026-11-05".to_owned(), &[1, 2]);
    cursor.after = "m/aa".to_owned();
    cursor.lost = 4;
    cursor.skip_unknown_store(&[2]);
    assert_eq!(
        (
            cursor.stage,
            cursor.store,
            cursor.after.as_str(),
            cursor.lost
        ),
        (Stage::List, 2, "", 0)
    );
    cursor.skip_unknown_store(&[2]);
    assert_eq!(cursor.store, 2);
    cursor.stage = Stage::Recount;
    cursor.skip_unknown_store(&[]);
    assert_eq!(cursor.stage, Stage::Recount);
}
