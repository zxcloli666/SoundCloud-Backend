use std::collections::HashMap;

use serde_json::{Value, json};

use super::merge::{
    dedupe_by_urn, local_after_window, local_is_enough, substitute, window_slice, wpages,
};
use super::query::LiveKind;

fn urns(n: usize) -> Vec<String> {
    (1..=n)
        .map(|id| format!("soundcloud:tracks:{id}"))
        .collect()
}

fn row(id: u32, title: &str, uploader: &str) -> Value {
    json!({
        "urn": format!("soundcloud:tracks:{id}"),
        "title": title,
        "user": {"username": uploader}
    })
}

fn sources(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item["_scd_search"]["source"].as_str().unwrap_or("untagged"))
        .collect()
}

#[test]
fn a_window_is_paged_in_full_slices_and_the_last_one_is_not_padded() {
    assert_eq!(wpages(0, 20), 0);
    assert_eq!(wpages(1, 20), 1);
    assert_eq!(wpages(40, 20), 2);
    assert_eq!(wpages(41, 20), 3);
    assert_eq!(wpages(7, 0), 7, "a zero limit must not divide by zero");

    let ids = urns(45);
    assert_eq!(window_slice(&ids, 0, 20), &ids[..20]);
    assert_eq!(window_slice(&ids, 1, 20), &ids[20..40]);
    assert_eq!(window_slice(&ids, 2, 20), &ids[40..45]);
    assert!(window_slice(&ids, 3, 20).is_empty());
    assert_eq!(window_slice(&ids, -1, 20).len(), 20);
}

#[test]
fn local_pages_continue_after_the_window_without_its_hits() {
    let window = urns(3);
    let local = vec![
        row(2, "two", "a"),
        row(9, "nine", "a"),
        json!({"title": "no urn"}),
        row(3, "three", "a"),
    ];
    let after = local_after_window(local, &window);
    let kept: Vec<&str> = after
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert_eq!(kept, ["nine", "no urn"]);
}

#[test]
fn a_local_serving_row_replaces_its_hit_and_a_hidden_one_drops_it() {
    let hits = vec![
        row(1, "live one", "x"),
        row(2, "live two", "x"),
        row(3, "live three", "x"),
        row(4, "live four", "x"),
    ];
    let serving: HashMap<String, Option<Value>> = HashMap::from([
        (
            "soundcloud:tracks:1".to_owned(),
            Some(row(1, "local one", "x")),
        ),
        ("soundcloud:tracks:2".to_owned(), None),
        (
            "soundcloud:tracks:3".to_owned(),
            Some(row(1, "the winner of three", "x")),
        ),
    ]);

    let page = substitute(hits, &serving);
    let titles: Vec<&str> = page
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        ["local one", "live four"],
        "a deleted row drops its hit and a superseded copy folds into a winner already shown"
    );
    assert_eq!(sources(&page), ["local", "soundcloud"]);
}

#[test]
fn dedupe_keeps_the_first_of_each_urn() {
    let items = vec![
        row(1, "first", "x"),
        row(2, "two", "x"),
        row(1, "again", "x"),
        json!({"title": "no urn"}),
    ];
    let deduped = dedupe_by_urn(items);
    let kept: Vec<&str> = deduped
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert_eq!(kept, ["first", "two", "no urn"]);
}

#[test]
fn ten_rows_with_a_named_match_keep_the_search_local() {
    let mut rows: Vec<Value> = (10..19)
        .map(|id| row(id, "something else", "nobody"))
        .collect();
    rows.push(row(42, "Lucid Dreams", "Juice WRLD"));
    assert!(local_is_enough(
        LiveKind::Tracks,
        "juice wrld lucid dreams",
        &rows
    ));
    assert!(local_is_enough(
        LiveKind::Tracks,
        "lucid dreams juice wrld",
        &rows
    ));
    assert!(local_is_enough(LiveKind::Tracks, "lucid dreams", &rows));
    assert!(!local_is_enough(LiveKind::Tracks, "lucid", &rows));
    assert!(
        !local_is_enough(LiveKind::Tracks, "juice wrld lucid dreams", &rows[1..]),
        "nine rows are thin whatever they hold"
    );

    let mut labelled = rows.clone();
    labelled[9] = json!({
        "urn": "soundcloud:tracks:7",
        "title": "Nightcall",
        "user": {"username": "Record Makers"},
        "metadata_artist": "Kavinsky"
    });
    assert!(local_is_enough(
        LiveKind::Tracks,
        "kavinsky nightcall",
        &labelled
    ));

    let mut buried = rows.clone();
    buried.insert(9, row(99, "filler", "nobody"));
    assert_eq!(buried[10]["title"], "Lucid Dreams");
    assert!(
        !local_is_enough(LiveKind::Tracks, "juice wrld lucid dreams", &buried),
        "a match below the top ten does not count"
    );
}

#[test]
fn users_and_playlists_go_live_below_five_rows() {
    let rows: Vec<Value> = (1..=5).map(|id| row(id, "x", "x")).collect();
    assert!(local_is_enough(LiveKind::Users, "anything", &rows));
    assert!(!local_is_enough(
        LiveKind::Playlists,
        "anything",
        &rows[..4]
    ));
}
