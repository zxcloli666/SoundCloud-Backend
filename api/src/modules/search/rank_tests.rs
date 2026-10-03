use super::rank::{Candidate, rank};
use super::terms::{QueryTerms, Span};

fn track(key: &str, title: &str, uploader: &str, plays: i64) -> Candidate {
    Candidate {
        key: key.to_owned(),
        title: title.to_owned(),
        uploader: Some(uploader.to_owned()),
        plays,
        ..Candidate::default()
    }
}

fn order(query: &str, span: Option<&Span>, candidates: Vec<Candidate>) -> Vec<String> {
    rank(&QueryTerms::parse(query), span, candidates)
        .scored
        .into_iter()
        .map(|scored| scored.key)
        .collect()
}

fn lucid_dreams() -> Vec<Candidate> {
    vec![
        track(
            "reupload",
            "Juice WRLD - Lucid Dreams",
            "lyricsvault",
            900_000_000,
        ),
        track("official", "Lucid Dreams", "Juice WRLD", 400_000_000),
        track(
            "lyrics",
            "Juice WRLD - Lucid Dreams (Lyrics)",
            "rapsounds",
            950_000_000,
        ),
    ]
}

#[test]
fn the_official_upload_beats_more_played_reuploads_in_either_word_order() {
    for query in ["juice wrld lucid dreams", "lucid dreams juice wrld"] {
        let ranking = rank(&QueryTerms::parse(query), None, lucid_dreams());
        assert_eq!(
            ranking.scored[0].key, "official",
            "{query}: {:#?}",
            ranking.scored
        );
        assert!(ranking.confident(), "{query}: {:#?}", ranking.top());
    }
}

#[test]
fn a_featured_artist_names_the_track() {
    let candidates = vec![
        track("band", "Rockstar", "some band", 1_000_000),
        track("savage", "21 Savage - a lot", "21 Savage", 300_000_000),
        track(
            "feat",
            "rockstar (feat. 21 Savage)",
            "Post Malone",
            800_000_000,
        ),
    ];
    assert_eq!(order("rockstar 21 savage", None, candidates)[0], "feat");
}

#[test]
fn metadata_artist_stands_in_for_a_label_uploader() {
    let mut official = track("official", "Nightcall", "Record Makers", 50_000_000);
    official.metadata_artist = Some("Kavinsky".to_owned());
    let candidates = vec![
        track(
            "hour",
            "Kavinsky - Nightcall (Drive OST) 1 hour",
            "loops",
            60_000_000,
        ),
        track("remix", "Nightcall (Remix)", "dj x", 70_000_000),
        official,
    ];
    let ranking = rank(&QueryTerms::parse("kavinsky nightcall"), None, candidates);
    assert_eq!(ranking.scored[0].key, "official");
    assert!(ranking.confident());
}

#[test]
fn a_cyrillic_artist_found_as_a_span_puts_its_own_upload_first() {
    let terms = QueryTerms::parse("король и шут кукла колдуньи");
    let span = terms
        .spans
        .iter()
        .find(|span| span.text == "король и шут")
        .cloned()
        .expect("the leading three words are a span");
    let mut official = track("official", "Кукла колдуньи", "Король и Шут", 30_000_000);
    official.spanned = true;
    let candidates = vec![
        track(
            "cover",
            "Король и Шут - Кукла Колдуньи (cover)",
            "guitarboy",
            40_000_000,
        ),
        track("fan", "Кукла колдуньи", "киш фан", 1_000_000),
        official,
    ];
    let ranking = rank(&terms, Some(&span), candidates);
    assert_eq!(ranking.scored[0].key, "official");
    assert!(ranking.confident());
}

#[test]
fn a_typo_in_the_artist_still_finds_the_song_but_is_not_confident() {
    let candidates = vec![
        track("cover", "Linkin Park - Numb (Cover)", "someone", 2_000_000),
        track("other", "Numb Little Bug", "Em Beihold", 300_000_000),
        track("official", "Numb", "Linkin Park", 900_000_000),
    ];
    let ranking = rank(&QueryTerms::parse("linkn park numb"), None, candidates);
    assert_eq!(ranking.scored[0].key, "official");
    assert!(!ranking.confident(), "{:#?}", ranking.top());
    assert!(ranking.weak());
}

#[test]
fn sped_up_and_nightcore_versions_sink_unless_asked_for() {
    let candidates = || {
        vec![
            track("sped", "Lucid Dreams (Sped Up)", "speedy", 900_000_000),
            track("nightcore", "Lucid Dreams (Nightcore)", "nc", 900_000_000),
            track("official", "Lucid Dreams", "Juice WRLD", 500_000_000),
        ]
    };
    assert_eq!(order("lucid dreams", None, candidates())[0], "official");
    assert_eq!(
        order("lucid dreams sped up", None, candidates())[0],
        "sped",
        "a query that asks for the sped up version gets it"
    );
}

#[test]
fn a_preview_and_junk_upload_lose_to_a_playable_one() {
    let mut preview = track("preview", "Blinding Lights", "The Weeknd", 900_000_000);
    preview.preview = true;
    let candidates = vec![
        preview,
        track(
            "boosted",
            "The Weeknd - Blinding Lights (8D Audio) bass boosted",
            "8d world",
            950_000_000,
        ),
        track("playable", "Blinding Lights", "The Weeknd", 100_000_000),
    ];
    assert_eq!(
        order("the weeknd blinding lights", None, candidates)[0],
        "playable"
    );
}

#[test]
fn a_live_hit_keeps_its_soundcloud_position_as_a_tie_breaker() {
    let mut first = track("first", "Ocean Eyes", "billie eilish", 0);
    first.sc_rank = Some(0);
    let mut later = track("later", "Ocean Eyes", "billie eilish", 0);
    later.sc_rank = Some(30);
    assert_eq!(
        order("billie eilish ocean eyes", None, vec![later, first]),
        ["first", "later"]
    );
}

#[test]
fn an_empty_pool_is_weak_and_never_confident() {
    let ranking = rank(&QueryTerms::parse("nothing at all"), None, Vec::new());
    assert!(ranking.weak());
    assert!(!ranking.confident());
}

#[test]
fn three_strong_titles_make_a_page_that_is_not_weak() {
    let candidates = (0..3)
        .map(|n| {
            track(
                &format!("t{n}"),
                "Numb",
                "Linkin Park",
                10_000_000 * (n + 1),
            )
        })
        .collect();
    let ranking = rank(&QueryTerms::parse("linkin park numb"), None, candidates);
    assert!(!ranking.weak());
}

#[test]
fn query_terms_keep_slots_spans_and_translit_variants() {
    let terms = QueryTerms::parse("Король и Шут — Кукла колдуньи");
    assert_eq!(terms.tokens, ["король", "и", "шут", "кукла", "колдуньи"]);
    assert_eq!(terms.slots, ["%колдуньи%", "%король%", "%кукла%", "%шут%"]);
    assert!(terms.span_names().contains(&"король и шут".to_owned()));
    assert!(terms.span_names().contains(&"korol i shut".to_owned()));
    let span = terms
        .spans
        .iter()
        .find(|span| span.text == "король и шут")
        .expect("span");
    assert_eq!(terms.rest_slots(span), ["%колдуньи%", "%кукла%"]);
    assert!(!terms.prefix_only);
    assert!(QueryTerms::parse("u2").prefix_only);
    assert_eq!(terms.hash().len(), 32);
}

#[test]
fn a_query_never_offers_more_than_five_splits() {
    let terms = QueryTerms::parse("a b c d e f");
    let span = terms.spans.iter().find(|span| span.len == 3).cloned();
    assert!(terms.splits(span.as_ref()).len() <= 5);
    assert_eq!(terms.splits(None)[0].artist, "");
}
