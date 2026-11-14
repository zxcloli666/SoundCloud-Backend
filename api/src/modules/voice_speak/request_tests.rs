use serde_json::{Value, json};

use super::*;

fn read(body: Value) -> Result<Line, String> {
    let body: SpeakBody = serde_json::from_value(body).expect("a speak body");
    body.into_line()
}

fn refused(body: Value) -> String {
    read(body).expect_err("the body is refused")
}

#[test]
fn a_known_voice_and_a_trimmed_text_make_a_line() {
    let line =
        read(json!({"voice": "moriarty", "text": "  Элементарно.  ", "keep": true})).unwrap();

    assert_eq!(
        line,
        Line {
            voice: "moriarty",
            text: "Элементарно.".to_owned(),
            keep: true,
        }
    );
    assert!(!read(json!({"voice": "loli", "text": "hi"})).unwrap().keep);
}

#[test]
fn an_empty_a_too_long_or_an_unknown_voice_line_is_refused() {
    assert_eq!(refused(json!({"voice": "loli"})), "text is empty");
    assert_eq!(
        refused(json!({"voice": "loli", "text": " \n "})),
        "text is empty"
    );
    assert_eq!(
        refused(json!({"voice": "loli", "text": "я".repeat(301)})),
        "text is longer than 300 characters"
    );
    assert!(read(json!({"voice": "loli", "text": format!(" {} ", "я".repeat(300))})).is_ok());
    assert_eq!(
        refused(json!({"voice": "nova", "text": "hi"})),
        "voice is not one of loli, moriarty"
    );
    assert!(refused(json!({"text": "hi"})).starts_with("voice"));
    assert!(refused(json!({"voice": "Loli", "text": "hi"})).starts_with("voice"));
}

#[test]
fn the_phrase_key_is_the_voice_then_the_hex_sha256_of_the_voice_a_newline_and_the_text() {
    let key = |body: Value| read(body).unwrap().phrase_key();

    assert_eq!(
        key(json!({"voice": "loli", "text": "hi"})),
        "loli/09b3f19cab3697f9ad75cf8288a3cb9547060020aed35477b8564a4b3a201082.ogg"
    );
    assert_eq!(
        key(json!({"voice": "moriarty", "text": " hi ", "keep": true})),
        "moriarty/e335a1430eaece14ff24340b90f22a4d70bbe5066583ec9543d688988432d412.ogg"
    );
}

#[test]
fn a_body_fits_the_longest_line_the_contract_allows() {
    let longest = json!({"voice": "moriarty", "text": "я".repeat(300), "keep": true});

    assert!(longest.to_string().len() < BODY_LIMIT);
}
