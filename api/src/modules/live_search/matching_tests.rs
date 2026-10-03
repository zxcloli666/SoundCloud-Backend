use serde_json::{Value, json};

use super::matching::{MatchReply, Wanted, judge};
use super::meta::{LiveMeta, LiveState};

fn hit(id: u64, title: &str, username: &str, duration: i64) -> Value {
    json!({
        "urn": format!("soundcloud:tracks:{id}"),
        "title": title,
        "duration": duration,
        "user": {"username": username}
    })
}

fn wanted() -> Wanted {
    Wanted::parse(Some("Juice WRLD"), Some("Lucid Dreams"), Some("239500")).expect("valid")
}

fn decide(items: Vec<Value>) -> MatchReply {
    let wanted = wanted();
    let judged = items
        .into_iter()
        .filter_map(|item| judge(&wanted, item, "soundcloud", None))
        .collect();
    MatchReply::decide(judged, LiveMeta::new(LiveState::Fresh, None))
}

#[test]
fn a_wanted_track_needs_a_title_and_keeps_a_sane_duration() {
    assert!(Wanted::parse(Some("artist"), None, None).is_err());
    assert!(Wanted::parse(Some("artist"), Some("  !! "), None).is_err());
    let wanted = Wanted::parse(Some("  Juice WRLD "), Some("Lucid Dreams"), Some("-5")).unwrap();
    assert_eq!(wanted.artist, "Juice WRLD");
    assert_eq!(wanted.duration_ms, None);
    assert_eq!(wanted.query(), "Juice WRLD Lucid Dreams");
    let long = "x".repeat(400);
    assert_eq!(
        Wanted::parse(None, Some(&long), None).unwrap().title.len(),
        128
    );
}

#[test]
fn the_same_song_of_the_same_length_is_a_match() {
    let reply = decide(vec![
        hit(1, "Lucid Dreams (Sped Up)", "Juice WRLD", 200_000),
        hit(2, "Lucid Dreams", "Juice WRLD", 239_000),
        hit(3, "Lucid Dreams", "a fan", 239_000),
    ]);
    let found = reply.found.expect("a confident match");
    assert_eq!(found.urn, "soundcloud:tracks:2");
    assert!(found.confidence >= 0.95, "{}", found.confidence);
    assert!(reply.candidates.len() <= 3);
}

#[test]
fn a_sped_up_version_or_a_preview_is_never_the_match() {
    let reply = decide(vec![hit(
        1,
        "Lucid Dreams (Sped Up)",
        "Juice WRLD",
        200_000,
    )]);
    assert!(reply.found.is_none());
    assert_eq!(reply.candidates.len(), 1, "it stays a candidate");

    let mut preview = hit(2, "Lucid Dreams", "Juice WRLD", 239_000);
    preview["access"] = json!("preview");
    let reply = decide(vec![preview.clone()]);
    assert!(
        reply.found.is_none(),
        "a thirty second preview is not the liked track"
    );

    let reply = decide(vec![preview, hit(3, "Lucid Dreams", "Juice WRLD", 241_000)]);
    assert_eq!(
        reply.found.map(|found| found.urn),
        Some("soundcloud:tracks:3".to_owned())
    );
}

#[test]
fn an_artist_credit_from_the_catalog_lifts_a_label_upload() {
    let wanted = wanted();
    let label = hit(4, "Lucid Dreams", "Grade A Productions", 239_000);
    let plain = judge(&wanted, label.clone(), "local", None).unwrap();
    let credited = judge(&wanted, label, "local", Some(1.0)).unwrap();
    assert!(plain.confidence < 0.8, "{}", plain.confidence);
    assert!(credited.confidence >= 0.95, "{}", credited.confidence);
}

#[test]
fn the_reply_lists_each_track_once_best_first_and_hides_the_payload() {
    let wanted = wanted();
    let judged = vec![
        judge(
            &wanted,
            hit(5, "Lucid Dreams", "a fan", 100_000),
            "local",
            None,
        )
        .unwrap(),
        judge(
            &wanted,
            hit(6, "Lucid Dreams", "Juice WRLD", 239_000),
            "local",
            None,
        )
        .unwrap(),
        judge(
            &wanted,
            hit(6, "Lucid Dreams", "Juice WRLD", 239_000),
            "soundcloud",
            None,
        )
        .unwrap(),
        judge(
            &wanted,
            hit(7, "Robbery", "Juice WRLD", 239_000),
            "local",
            None,
        )
        .unwrap(),
        judge(
            &wanted,
            hit(8, "Wishing Well", "Juice WRLD", 239_000),
            "local",
            None,
        )
        .unwrap(),
    ];
    let reply = MatchReply::decide(judged, LiveMeta::new(LiveState::Local, None));
    let urns: Vec<&str> = reply
        .candidates
        .iter()
        .map(|one| one.urn.as_str())
        .collect();
    assert_eq!(urns.len(), 3);
    assert_eq!(urns[0], "soundcloud:tracks:6");
    assert_eq!(
        urns.iter()
            .filter(|urn| **urn == "soundcloud:tracks:6")
            .count(),
        1
    );

    let body = serde_json::to_value(&reply).unwrap();
    assert_eq!(body["match"]["urn"], "soundcloud:tracks:6");
    assert_eq!(body["match"]["source"], "local");
    assert_eq!(body["live"]["state"], "local");
    assert!(body["match"].get("item").is_none());
    assert!(body["candidates"][0]["confidence"].is_number());
}
