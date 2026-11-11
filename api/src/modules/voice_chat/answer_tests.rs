use serde_json::json;

use super::*;

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
        "bad \u{fffd} digits"
    );
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
