use std::collections::HashSet;
use std::time::Duration;

use super::prompt::{self, Pick};
use super::rank::{Track, decide, duration_closeness, is_borderline, rank, score};
use super::request::{MatchCandidate, MatchTarget};
use super::similarity::{ratio, token_set};
use crate::handlers::ai_store::RpcSource;
use crate::handlers::metadata::Normalizer;
use crate::llm::test_support::answer_fields;

fn target(title: &str) -> MatchTarget {
    MatchTarget {
        artist: "Billie Eilish".to_owned(),
        title: title.to_owned(),
    }
}

fn offers(rows: &[(u32, &str, &str, Option<f64>)]) -> Vec<MatchCandidate> {
    rows.iter()
        .map(|(id, artist, title, duration_sec)| MatchCandidate {
            id: *id,
            artist: (*artist).to_owned(),
            title: (*title).to_owned(),
            duration_sec: *duration_sec,
        })
        .collect()
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

fn pick(body: &str) -> anyhow::Result<Pick> {
    Ok(serde_json::from_str(body)?)
}

fn borderline() -> Vec<MatchCandidate> {
    offers(&[
        (1, "Billie Eilish", "Ocean Eyes", Some(200.0)),
        (2, "billie", "Ocean Eyes", Some(201.0)),
        (3, "Billie Eilish", "Ocean Eyes (Remix)", Some(200.0)),
    ])
}

#[test]
fn a_clear_winner_is_answered_without_the_llm() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = offers(&[
        (1, "Someone", "Different Song", Some(200.0)),
        (2, "Billie Eilish", "Ocean Eyes", Some(200.0)),
    ]);

    let ranked = rank(&normalizer, &target("Ocean Eyes"), &candidates);
    let verdict = decide(&ranked);

    assert_eq!(verdict.match_id, Some(2));
    assert!(verdict.confidence >= 0.85);
    assert_eq!(verdict.source, RpcSource::Deterministic);
    assert!(!is_borderline(&ranked));
    Ok(())
}

#[test]
fn veto_removes_other_versions() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = offers(&[
        (1, "Billie Eilish", "Ocean Eyes (Skrillex Remix)", None),
        (2, "Billie Eilish", "Ocean Eyes (Live)", None),
        (3, "Billie Eilish", "Ocean Eyes - Sped Up", None),
    ]);

    let verdict = decide(&rank(&normalizer, &target("Ocean Eyes"), &candidates));

    assert_eq!((verdict.match_id, verdict.confidence), (None, 0.0));
    Ok(())
}

#[test]
fn a_wanted_version_matches_the_same_version() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = offers(&[
        (1, "Billie Eilish", "Ocean Eyes", None),
        (2, "Billie Eilish", "Ocean Eyes (Live)", None),
    ]);

    let verdict = decide(&rank(
        &normalizer,
        &target("Ocean Eyes (Live)"),
        &candidates,
    ));

    assert_eq!(verdict.match_id, Some(2));
    Ok(())
}

#[test]
fn a_weak_best_is_no_match() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = offers(&[(1, "Nobody", "Something Else", Some(90.0))]);

    let ranked = rank(&normalizer, &target("Ocean Eyes"), &candidates);
    let verdict = decide(&ranked);

    assert_eq!(verdict.match_id, None);
    assert!(verdict.confidence < 0.5);
    assert!(!is_borderline(&ranked));
    Ok(())
}

#[test]
fn a_tie_is_null_with_the_best_confidence() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = offers(&[
        (1, "Billie Eilish", "Ocean Eyes", Some(200.0)),
        (2, "Billie Eilish", "Ocean Eyes", Some(200.0)),
    ]);

    let verdict = decide(&rank(&normalizer, &target("Ocean Eyes"), &candidates));

    assert_eq!(verdict.match_id, None);
    assert!(verdict.confidence > 0.9);
    assert_eq!(verdict.source, RpcSource::Deterministic);
    Ok(())
}

#[test]
fn a_borderline_pick_is_asked_with_the_shortlist_in_the_worker_words() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = borderline();
    let wanted = target("Ocean Eyes");

    let ranked = rank(&normalizer, &wanted, &candidates);
    let asked = prompt::request(&wanted, &ranked, Duration::from_secs(10));

    assert!(is_borderline(&ranked));
    assert_eq!(
        asked.system.joined(),
        "Pick the candidate that is the same recording as the target, or null if none is.\n\
         A remix, live, acoustic, instrumental, sped up or slowed version is a different\n\
         recording. Candidate artists are often uploader names, not the performer.\n\
         Answer with exactly one JSON object and nothing else, no markdown and no text around it:\n\
         {\"match_id\": <id of the chosen candidate or null>, \"confidence\": <number from 0 to 1>}"
    );
    assert_eq!(
        asked
            .messages
            .first()
            .map(|message| message.content.as_str()),
        Some(
            "target: artist='Billie Eilish' title='Ocean Eyes'\n\
             candidates:\n\
             - id=1 artist='Billie Eilish' title='Ocean Eyes' duration_sec=200.0\n\
             - id=2 artist='billie' title='Ocean Eyes' duration_sec=201.0"
        )
    );
    assert_eq!(asked.deadline, Some(Duration::from_secs(10)));
    assert_eq!(asked.budget_user, None);
    Ok(())
}

#[test]
fn every_answer_field_is_named_in_the_prompt() -> anyhow::Result<()> {
    let system = prompt::request(&target("Ocean Eyes"), &[], Duration::from_secs(10))
        .system
        .joined();
    let fields = answer_fields(&serde_json::to_value(prompt::schema())?);
    assert_eq!(fields.len(), 2);
    for field in fields {
        assert!(system.contains(&format!("\"{field}\"")), "{field}");
    }
    Ok(())
}

#[test]
fn the_answer_schema_is_the_worker_schema() -> anyhow::Result<()> {
    assert_eq!(
        serde_json::to_value(prompt::schema())?,
        serde_json::json!({
            "type": "object",
            "properties": {
                "match_id": {"anyOf": [{"type": "integer"}, {"type": "null"}]},
                "confidence": {"type": "number"},
            },
            "required": ["match_id", "confidence"],
            "additionalProperties": false,
        })
    );
    Ok(())
}

#[test]
fn the_llm_may_pick_inside_the_shortlist_or_answer_no_match() -> anyhow::Result<()> {
    let allowed = HashSet::from([1, 2]);

    let picked = prompt::read(&pick(r#"{"match_id": 2, "confidence": 0.8}"#)?, &allowed)
        .ok_or_else(|| anyhow::anyhow!("rejected"))?;
    let none = prompt::read(&pick(r#"{"match_id": null, "confidence": 0.7}"#)?, &allowed)
        .ok_or_else(|| anyhow::anyhow!("rejected"))?;

    assert_eq!(
        (picked.match_id, picked.confidence, picked.source),
        (Some(2), 0.8, RpcSource::Llm)
    );
    assert_eq!((none.match_id, none.confidence), (None, 0.7));
    Ok(())
}

#[test]
fn ids_outside_the_shortlist_or_vetoed_are_ungrounded() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let candidates = borderline();
    let ranked = rank(&normalizer, &target("Ocean Eyes"), &candidates);
    let allowed: HashSet<u32> = ranked.iter().map(|scored| scored.candidate.id).collect();

    for picked in [7, 3] {
        let reply = pick(&format!(r#"{{"match_id": {picked}, "confidence": 0.99}}"#))?;

        assert!(prompt::read(&reply, &allowed).is_none(), "{picked}");
    }
    assert_eq!(allowed, HashSet::from([1, 2]));
    Ok(())
}

#[test]
fn malformed_picks_are_rejected() -> anyhow::Result<()> {
    let allowed = HashSet::from([1]);

    assert!(pick(r#"{"match_id": true, "confidence": 0.9}"#).is_err());
    assert!(pick(r#"{"match_id": 1.0, "confidence": 0.9}"#).is_err());
    assert!(pick(r#"{"match_id": 1, "confidence": true}"#).is_err());
    assert!(pick(r#"{"match_id": -1, "confidence": 0.5}"#).is_err());
    assert!(prompt::read(&pick(r#"{"match_id": 1, "confidence": 1.5}"#)?, &allowed).is_none());
    Ok(())
}

#[test]
fn the_u32_edge_id_and_fifty_candidates_are_accepted() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let mut rows: Vec<(u32, String, String, Option<f64>)> = (0..49)
        .map(|id| (id, "Someone".to_owned(), format!("Song {id}"), Some(100.0)))
        .collect();
    rows.push((
        u32::MAX,
        "Billie Eilish".to_owned(),
        "Ocean Eyes".to_owned(),
        Some(200.0),
    ));
    let candidates: Vec<MatchCandidate> = rows
        .into_iter()
        .map(|(id, artist, title, duration_sec)| MatchCandidate {
            id,
            artist,
            title,
            duration_sec,
        })
        .collect();

    let verdict = decide(&rank(&normalizer, &target("Ocean Eyes"), &candidates));

    assert_eq!(verdict.match_id, Some(u32::MAX));
    Ok(())
}

#[test]
fn score_weights_artist_title_and_duration() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let track = |artist: &str, title: &str, duration: Option<f64>| {
        Track::read(&normalizer, artist, title, duration)
    };
    let wanted = track("Billie Eilish", "Ocean Eyes", Some(200.0));

    assert!(near(
        score(&wanted, &track("BILLIE EILISH", "ocean eyes!", Some(200.0))),
        1.0
    ));
    assert!(near(
        score(&wanted, &track("Billie Eilish", "Ocean Eyes", None)),
        0.95
    ));
    assert!(near(
        score(&wanted, &track("Billie Eilish", "Ocean Eyes", Some(400.0))),
        0.9
    ));
    Ok(())
}

#[test]
fn uploader_style_candidates_use_the_artist_from_the_title() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let wanted = Track::read(&normalizer, "Billie Eilish", "Ocean Eyes", None);
    let reupload = Track::read(
        &normalizer,
        "Nightcore Vibes",
        "Billie Eilish - Ocean Eyes",
        None,
    );

    assert!(score(&wanted, &reupload) > 0.9);
    Ok(())
}

#[test]
fn token_set_ignores_word_order() {
    assert!(near(
        token_set("Ocean Eyes Billie", "Billie Ocean Eyes"),
        1.0
    ));
    assert_eq!(token_set("", "anything"), 0.0);
}

#[test]
fn duration_closeness_follows_the_tolerance() {
    for (wanted, offered, closeness) in [
        (Some(200.0), Some(203.0), 1.0),
        (Some(200.0), Some(225.0), 0.0),
        (Some(200.0), Some(214.5), 0.5),
        (None, Some(200.0), 0.5),
    ] {
        assert!(near(duration_closeness(wanted, offered), closeness));
    }
}

#[test]
fn ratio_is_the_python_difflib_ratio() {
    let shuffled: String = (0..220)
        .filter_map(|index| char::from_u32(0x41 + (index * 7) % 26))
        .collect();
    for (left, right, expected) in [
        ("abcd".to_owned(), "bcde".to_owned(), 0.75),
        (
            " abcd".to_owned(),
            "abcd abcd".to_owned(),
            0.7142857142857143,
        ),
        (
            "ocean eyes".to_owned(),
            "billie ocean eyes".to_owned(),
            0.7407407407407407,
        ),
        (
            "billie eilish".to_owned(),
            "billie".to_owned(),
            0.631578947368421,
        ),
        (
            "private thread currentthread".to_owned(),
            "private volatile thread currentthread".to_owned(),
            0.8615384615384616,
        ),
        ("ливень".to_owned(), "ливни".to_owned(), 0.7272727272727273),
        (
            "eilish billie".to_owned(),
            "billie eilish".to_owned(),
            0.46153846153846156,
        ),
        (
            "kendrick lamar luther".to_owned(),
            "luther sza".to_owned(),
            0.3870967741935484,
        ),
        (String::new(), "abc".to_owned(), 0.0),
        (String::new(), String::new(), 1.0),
        (format!("x{}", "a".repeat(10)), "a".repeat(200), 0.0),
        ("a".repeat(10), "a".repeat(200), 0.09523809523809523),
        (
            "the quick brown fox jumps over the lazy dog ".repeat(3),
            "lorem ipsum dolor sit amet consectetur ".repeat(6),
            0.0,
        ),
        (
            "abracadabra ".repeat(20),
            "cadabra abracad ".repeat(15),
            0.0,
        ),
        (
            "zqeeeeexyz".to_owned(),
            format!("{}xyzzq", "e".repeat(240)),
            0.06274509803921569,
        ),
        (
            "kendrick lamar ".repeat(14),
            "kendrick lamar sza luther gnx ".repeat(8),
            0.06666666666666667,
        ),
        (
            "0123456789abcdefghij".repeat(2),
            format!("{shuffled}0123456789"),
            0.07407407407407407,
        ),
        (
            "aaaaab".to_owned(),
            format!("b{}c", "a".repeat(199)),
            0.00966183574879227,
        ),
    ] {
        assert_eq!(ratio(&left, &right), expected, "{left:?} {right:?}");
    }
}
