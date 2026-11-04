use axum::http::{HeaderMap, HeaderValue};
use serde_json::json;

use super::alice::{AliceRequest, pong, spoken, start_linking};
use super::converse::Said;
use super::salut::{SalutRequest, admitted, answer, authorized};
use axum::http::StatusCode;

const ALICE_SAMPLE: &str = r#"{
  "meta": {
    "locale": "ru-RU",
    "timezone": "Europe/Moscow",
    "client_id": "ru.yandex.searchplugin/5.80 (Samsung Galaxy; Android 4.4)",
    "interfaces": {"screen": {}, "payments": {}, "account_linking": {}}
  },
  "session": {
    "message_id": 4,
    "session_id": "2eac4854-fce721f3-b845abba-20d60",
    "skill_id": "3ad36498-f5rd-4079-a14b-788652932056",
    "user": {
      "user_id": "6C91DA5198D1758C6A9F63A7C5CDDF09359F683B13A18A151FBF4C8B092BB0C2",
      "access_token": "AgAAAAAB4vpbAAApoR1oaCd5yR6eiXSHqOGT8dT"
    },
    "application": {"application_id": "47C73714B580ED2469056E71081159529FFC676A4E5B059D684100A3A1C6F1CC"},
    "user_id": "47C73714B580ED2469056E71081159529FFC676A4E5B059D684100A3A1C6F1CC",
    "new": false
  },
  "request": {
    "command": "включи следующий трек",
    "original_utterance": "Включи следующий трек",
    "nlu": {"tokens": ["включи", "следующий", "трек"], "entities": [], "intents": {}},
    "markup": {"dangerous_context": false},
    "type": "SimpleUtterance"
  },
  "version": "1.0"
}"#;

const SALUT_SAMPLE: &str = r#"{
  "sessionId": "86024848-c12b-4056-b58b-93c69b412314",
  "messageId": 12,
  "uuid": {"userChannel": "B2C", "sub": "d2d6da62-6bdd-452b-b5dd-a145090075ba", "userId": "123"},
  "messageName": "MESSAGE_TO_SKILL",
  "payload": {
    "app_info": {"projectId": "p", "frontendType": "APK"},
    "intent": "run_app",
    "projectName": "lolya",
    "device": {"platformType": "ANDROID", "surface": "SBOL"},
    "new_session": false,
    "character": {"id": "sber", "name": "Сбер", "gender": "male", "appeal": "official"},
    "message": {
      "original_text": "один два три четыре пять шесть",
      "normalized_text": "один два три четыре пять шесть",
      "asr_normalized_message": "1 2 3 4 5 6",
      "entities": {},
      "tokenized_elements_list": []
    }
  }
}"#;

#[test]
fn an_alice_request_parses_and_prefers_the_original_utterance() {
    let request: AliceRequest = serde_json::from_str(ALICE_SAMPLE).unwrap();
    assert_eq!(
        request.session.skill_id,
        "3ad36498-f5rd-4079-a14b-788652932056"
    );
    assert_eq!(request.utterance(), "Включи следующий трек");
    assert!(!request.session.new);
}

#[test]
fn the_alice_token_comes_from_the_header_or_the_body() {
    let request: AliceRequest = serde_json::from_str(ALICE_SAMPLE).unwrap();
    let mut headers = HeaderMap::new();
    assert_eq!(
        request.token(&headers).as_deref(),
        Some("AgAAAAAB4vpbAAApoR1oaCd5yR6eiXSHqOGT8dT")
    );
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer from-header"),
    );
    assert_eq!(request.token(&headers).as_deref(), Some("from-header"));
    let bare: AliceRequest =
        serde_json::from_str(r#"{"session":{"new":true},"request":{}}"#).unwrap();
    assert_eq!(bare.token(&HeaderMap::new()), None);
    assert_eq!(bare.utterance(), "");
}

#[test]
fn a_missing_original_utterance_falls_back_to_the_command() {
    let request: AliceRequest = serde_json::from_str(
        r#"{"session":{},"request":{"command":"пауза","original_utterance":""}}"#,
    )
    .unwrap();
    assert_eq!(request.utterance(), "пауза");
}

#[test]
fn alice_replies_have_the_documented_shape() {
    let said = Said {
        text: "Готово".into(),
        speech: "Готово".into(),
    };
    let body = spoken(&said, true);
    assert_eq!(body["response"]["text"], "Готово");
    assert_eq!(body["response"]["end_session"], true);
    assert_eq!(body["version"], "1.0");

    let linking = start_linking("Свяжите аккаунт");
    assert_eq!(
        linking["response"]["directives"],
        json!({"start_account_linking": {}})
    );
    assert_eq!(linking["response"]["end_session"], false);

    assert_eq!(pong()["response"]["text"], "pong");
}

#[test]
fn a_salut_request_parses_and_echoes_the_message_id() {
    let request: SalutRequest = serde_json::from_str(SALUT_SAMPLE).unwrap();
    assert_eq!(request.message_name, "MESSAGE_TO_SKILL");
    assert_eq!(request.sub(), Some("d2d6da62-6bdd-452b-b5dd-a145090075ba"));
    assert_eq!(request.utterance(), "один два три четыре пять шесть");
    assert_eq!(request.spoken_code(), "123456");

    let reply = answer(&request, &Said::plain("Привет"), false);
    assert_eq!(reply["messageName"], "ANSWER_TO_USER");
    assert_eq!(reply["messageId"], 12);
    assert_eq!(reply["sessionId"], "86024848-c12b-4056-b58b-93c69b412314");
    assert_eq!(reply["uuid"]["sub"], "d2d6da62-6bdd-452b-b5dd-a145090075ba");
    assert_eq!(reply["payload"]["pronounceText"], "Привет");
    assert_eq!(reply["payload"]["items"][0]["bubble"]["text"], "Привет");
    assert_eq!(reply["payload"]["finished"], false);
}

#[test]
fn a_code_is_read_from_the_words_when_the_asr_text_has_no_digits() {
    let mut request: SalutRequest = serde_json::from_str(SALUT_SAMPLE).unwrap();
    request
        .payload
        .message
        .as_mut()
        .unwrap()
        .asr_normalized_message = String::new();
    assert_eq!(request.spoken_code(), "123456");
}

#[test]
fn a_run_app_request_without_a_message_still_parses() {
    let request: SalutRequest = serde_json::from_str(
        r#"{"sessionId":"s","messageId":0,"uuid":{"sub":"x"},"messageName":"RUN_APP","payload":{"intent":"run_app"}}"#,
    )
    .unwrap();
    assert_eq!(request.utterance(), "");
    assert_eq!(request.spoken_code(), "");
}

#[test]
fn the_shared_salut_token_is_taken_from_either_header_or_the_path_and_never_empty() {
    let mut headers = HeaderMap::new();
    assert!(!authorized(&headers, None, "secret"));
    assert!(authorized(&headers, Some("secret"), "secret"));
    assert!(!authorized(&headers, Some("secreT"), "secret"));
    assert!(!authorized(&headers, Some(""), ""));
    headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
    assert!(authorized(&headers, None, "secret"));
    assert!(!authorized(&headers, None, "other"));
    assert!(!authorized(&headers, None, ""));

    let mut custom = HeaderMap::new();
    custom.insert("x-voice-token", HeaderValue::from_static("secret"));
    assert!(authorized(&custom, None, "secret"));

    let mut blank = HeaderMap::new();
    blank.insert("authorization", HeaderValue::from_static("Bearer "));
    assert!(!authorized(&blank, None, ""));
}

#[test]
fn salut_admits_by_token_by_project_or_not_at_all() {
    let request: SalutRequest = serde_json::from_str(SALUT_SAMPLE).unwrap();
    assert_eq!(request.project_id(), "p");
    let none = HeaderMap::new();

    assert_eq!(
        admitted("", "", &none, Some("x"), &request),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    );
    assert_eq!(
        admitted("tok", "", &none, None, &request),
        Err(StatusCode::UNAUTHORIZED)
    );
    assert_eq!(admitted("tok", "", &none, Some("tok"), &request), Ok(()));
    assert_eq!(admitted("", "p", &none, None, &request), Ok(()));
    assert_eq!(
        admitted("", "other", &none, None, &request),
        Err(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        admitted("tok", "other", &none, Some("tok"), &request),
        Err(StatusCode::FORBIDDEN)
    );
    assert_eq!(admitted("tok", "p", &none, Some("tok"), &request), Ok(()));

    let bare: SalutRequest = serde_json::from_str(
        r#"{"sessionId":"s","messageId":0,"uuid":{"sub":"x"},"messageName":"RUN_APP","payload":{}}"#,
    )
    .unwrap();
    assert_eq!(bare.project_id(), "");
    assert_eq!(
        admitted("", "p", &none, None, &bare),
        Err(StatusCode::FORBIDDEN)
    );
}
