use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;

use super::*;

const SLICINGS: u64 = 300;

fn whole(document: &str) -> Vec<Piece> {
    Answer::default().push(document)
}

fn by_symbol(document: &str) -> (Vec<Value>, String) {
    let mut answer = Answer::default();
    let mut actions = Vec::new();
    let mut said = String::new();
    for symbol in document.chars() {
        for piece in answer.push(&symbol.to_string()) {
            match piece {
                Piece::Action(action) => actions.push(action),
                Piece::Say(text) => said.push_str(&text),
            }
        }
    }
    (actions, said)
}

fn sliced(document: &str, seed: u64) -> (Vec<Value>, String) {
    let mut random = StdRng::seed_from_u64(seed);
    let symbols: Vec<char> = document.chars().collect();
    let mut answer = Answer::default();
    let mut actions = Vec::new();
    let mut said = String::new();
    let mut at = 0;
    while at < symbols.len() {
        let end = (at + random.gen_range(1..=7)).min(symbols.len());
        let piece: String = symbols[at..end].iter().collect();
        for piece in answer.push(&piece) {
            match piece {
                Piece::Action(action) => actions.push(action),
                Piece::Say(text) => said.push_str(&text),
            }
        }
        at = end;
    }
    (actions, said)
}

fn said(document: &str) -> String {
    by_symbol(document).1
}

#[test]
fn actions_come_out_one_by_one_before_the_speech() {
    let document = r#"{"actions":[{"name":"volume_set","args":[{"name":"level","value":60}]},{"name":"next_track","args":[]}],"say":"Louder, and the next one."}"#;
    assert_eq!(
        whole(document),
        [
            Piece::Action(json!({"name": "volume_set", "args": [{"name": "level", "value": 60}]})),
            Piece::Action(json!({"name": "next_track", "args": []})),
            Piece::Say("Louder, and the next one.".to_owned()),
        ]
    );
}

#[test]
fn an_action_is_reported_as_soon_as_its_last_brace_arrives() {
    let mut answer = Answer::default();
    assert_eq!(answer.push(r#"{"actions":[{"name":"pause","ar"#), []);
    assert_eq!(
        answer.push(r#"gs":[]}"#),
        [Piece::Action(json!({"name": "pause", "args": []}))]
    );
    assert_eq!(
        answer.push(r#"],"say":"Pau"#),
        [Piece::Say("Pau".to_owned())]
    );
    assert_eq!(answer.push(r#"sed."}"#), [Piece::Say("sed.".to_owned())]);
    assert_eq!(answer.push("trailing words"), []);
}

#[test]
fn the_answer_reads_the_same_however_it_is_cut() {
    let document = r#" { "actions" : [ {"name":"play","args":{"query":"a \"quoted\" ] } name"}} , {"name":"like"} ] , "say" : "Нашла: «Ночь» — включаю!\nГотово." } "#;
    let (actions, said) = by_symbol(document);
    assert_eq!(
        actions,
        [
            json!({"name": "play", "args": {"query": "a \"quoted\" ] } name"}}),
            json!({"name": "like"}),
        ]
    );
    assert_eq!(said, "Нашла: «Ночь» — включаю!\nГотово.");
    let pieces = whole(document);
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[2], Piece::Say(said));
}

#[test]
fn escapes_in_the_speech_are_decoded_even_when_split() {
    assert_eq!(said(r#"{"say":"a\"b\\c\/d\te\r\nf"}"#), "a\"b\\c/d\te\r\nf");
    assert_eq!(said(r#"{"say":"привет"}"#), "привет");
    assert_eq!(said(r#"{"say":"ok 🎵 go"}"#), "ok 🎵 go");
    assert_eq!(said(r#"{"say":"lone \ud83c end"}"#), "lone \u{fffd} end");
    assert_eq!(said(r#"{"say":"low \udfb5 only"}"#), "low \u{fffd} only");
    assert_eq!(
        said(r#"{"say":"bad \uZZZZ digits"}"#),
        "bad \u{fffd}ZZZZ digits"
    );
}

#[test]
fn a_broken_unicode_escape_becomes_one_replacement_and_the_rest_is_kept() {
    assert_eq!(said(r#"{"say":"a\uя rest"}"#), "a\u{fffd}я rest");
    assert_eq!(said(r#"{"say":"a\u12я4 rest"}"#), "a\u{fffd}я4 rest");
    assert_eq!(said(r#"{"say":"a\u0я"}"#), "a\u{fffd}я");
    assert_eq!(
        by_symbol(r#"{"say":"cut \u12","actions":[{"name":"stop"}]}"#),
        (vec![json!({"name": "stop"})], "cut \u{fffd}".to_owned()),
        "a quote right after a broken escape still closes the speech"
    );
    assert_eq!(said(r#"{"say":"\ud83c\uя"}"#), "\u{fffd}\u{fffd}я");
    assert_eq!(said(r#"{"say":"\uD83C\uDFB5 \u00E9"}"#), "🎵 é");
}

#[test]
fn speech_before_the_actions_and_unknown_keys_are_handled() {
    let document = r#"{"mood":{"say":"not this","list":[1,"]",{"a":"}"}]},"say":"First.","note":null,"count":12,"actions":[{"name":"stop"}],"flag":true}"#;
    assert_eq!(
        whole(document),
        [
            Piece::Say("First.".to_owned()),
            Piece::Action(json!({"name": "stop"})),
        ]
    );
    assert_eq!(
        by_symbol(document),
        (vec![json!({"name": "stop"})], "First.".to_owned())
    );
}

#[test]
fn elements_that_are_not_objects_still_come_out_for_the_caller_to_drop() {
    let (actions, said) =
        by_symbol(r#"{"actions":[1, "play", null, {"name":"stop"}, [2]],"say":"ok"}"#);
    assert_eq!(
        actions,
        [
            json!(1),
            json!("play"),
            json!(null),
            json!({"name": "stop"}),
            json!([2])
        ]
    );
    assert_eq!(said, "ok");
}

#[test]
fn values_of_another_type_under_the_known_keys_are_skipped() {
    assert_eq!(whole(r#"{"actions":"none","say":null}"#), []);
    assert_eq!(whole(r#"{"actions":{"name":"stop"},"say":["a","b"]}"#), []);
    assert_eq!(
        whole(r#"{"say":"only speech"}"#),
        [Piece::Say("only speech".to_owned())]
    );
    assert_eq!(whole(r#"{"actions":[],"say":""}"#), []);
    assert_eq!(whole("{}"), []);
}

#[test]
fn any_slicing_of_a_valid_answer_reads_as_serde_json_reads_it() {
    let documents = [
        r#"{"actions":[{"name":"play","args":[{"name":"query","value":"\u043d\u043e\u0447\u044c \ud83c\udfb5 \"q\" \\ ]}"}]},{"name":"like","args":[]}],"say":"\u041f\u0440\u0438\u0432\u0435\u0442 \ud83c\udfb5\n\t\/ \u00e9nd"}"#,
        r#"{ "say" : "first \"quoted\" \\u0041 is no escape" , "actions" : [ { "name" : "volume" , "args" : [ { "name" : "value" , "value" : 60 } ] } ] , "extra" : {"say": "not this", "list": ["]", "}"]} }"#,
        r#"{"actions":[],"say":"Нашла «Ночь» — 🎵 включаю"}"#,
        r#"{"actions":[{"name":"remember","args":[{"name":"text","value":"\uD83D\uDE00 smile"}]}],"say":"\uD83D\uDE00"}"#,
    ];
    for document in documents {
        let parsed: Value = serde_json::from_str(document).expect("a valid document");
        let expected = (
            parsed["actions"].as_array().cloned().unwrap_or_default(),
            parsed["say"].as_str().unwrap_or_default().to_owned(),
        );
        assert_eq!(by_symbol(document), expected, "{document}");
        for seed in 0..SLICINGS {
            assert_eq!(sliced(document, seed), expected, "seed {seed}: {document}");
        }
    }
}

#[test]
fn any_slicing_of_a_broken_escape_reads_the_same_and_keeps_what_follows() {
    let documents = [
        (
            r#"{"actions":[],"say":"one \uZZ two \x three"}"#,
            "one \u{fffd}ZZ two x three",
        ),
        (
            r#"{"actions":[],"say":"lone \ud83c end \udfb5 low"}"#,
            "lone \u{fffd} end \u{fffd} low",
        ),
        (r#"{"actions":[],"say":"cut \u12"}"#, "cut \u{fffd}"),
        (r#"{"actions":[],"say":"\u0041\u00я!"}"#, "A\u{fffd}я!"),
    ];
    for (document, expected) in documents {
        assert!(
            serde_json::from_str::<Value>(document).is_err(),
            "{document}"
        );
        assert_eq!(said(document), expected);
        for seed in 0..SLICINGS {
            assert_eq!(
                sliced(document, seed).1,
                expected,
                "seed {seed}: {document}"
            );
        }
    }
}
