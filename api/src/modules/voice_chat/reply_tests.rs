use std::sync::Mutex;

use llm_ladder::Hosting;
use serde_json::{Value, json};

use super::*;
use crate::modules::voice_chat::VoiceChat;
use crate::modules::voice_chat::request::ChatBody;
use crate::modules::voice_chat::testing::{Asked, Heard, Unwatched, hear, local_only, tools};
use crate::modules::voice_chat::tools::ToolSpec;

const LOUDER: &str = "{\"actions\":[{\"name\":\"volume_set\",\"args\":[{\"name\":\"level\",\"value\":90}]}],\"say\":\"\"}";
const LOUDER_ALOUD: &str = "{\"actions\":[{\"name\":\"volume_set\",\"args\":[{\"name\":\"level\",\"value\":90}]}],\"say\":\"Громче.\"}";
const UNKNOWN: &str = "{\"actions\":[{\"name\":\"erase_library\",\"args\":[]}],\"say\":\"\"}";
const SPOKEN: &str = "{\"say\":\"Привет.\"}";
const SPOKEN_WITH_NO_ACTIONS: &str = "{\"actions\":[],\"say\":\"Привет.\"}";
const SILENT: &str = "{\"actions\":[],\"say\":\" \"}";
const NOTHING: &str = "{\"actions\":[],\"say\":\"\"}";
const NOTHING_SO_FAR: &str = "{\"actions\":[],\"say\":\"\"";
const NO_WORD: &str = "{\"say\":\"\"}";
const NAMED: &str = "{\"actions\":[],\"say\":\"Включаю [[Psychosis|Психосис]].\"}";
const ONLY_A_NAME: &str = "{\"actions\":[],\"say\":\"[[Psychosis|Психосис]]\"}";
const A_NAME_SO_FAR: &str = "{\"actions\":[],\"say\":\"[[Psychosis|Пси";

fn offered() -> Arc<Offered> {
    Offered::new(tool_set(), false)
}

fn overhearing() -> Arc<Offered> {
    Offered::new(tool_set(), true)
}

fn tool_set() -> Tools {
    let specs: Vec<ToolSpec> = serde_json::from_value(tools()).expect("a tool list");
    Tools::read(specs, 8).expect("a tool set")
}

fn louder() -> Value {
    json!({"name": "volume_set", "args": [{"name": "level", "value": 90}]})
}

async fn streamed(answer: &'static str, quiet: bool) -> Heard {
    let asked = Arc::new(Asked {
        answer,
        payloads: Mutex::default(),
    });
    let hosting = Hosting {
        jetstream: None,
        worker: Some(asked),
        observer: Arc::new(Unwatched),
    };
    let chat = VoiceChat::hosted(&local_only(""), hosting).expect("the chat ladder");
    let live = chat.live().expect("a live chat");
    let body = json!({"text": "включи психозис", "tools": tools(), "quiet": quiet});
    let body: ChatBody = serde_json::from_value(body).expect("a chat body");
    let turn = body.into_turn(live.limits()).expect("a valid turn");
    let request = live
        .request(&turn, "names")
        .expect("a prompt within the cap");
    let response = live.answer(request, turn).await.expect("an open step");
    hear(response).await.expect("a stream")
}

#[test]
fn a_full_answer_is_accepted_for_a_fitting_action_or_for_speech() {
    let offered = offered();
    assert!(offered.accepts(Form::Full, LOUDER));
    assert!(offered.accepts(Form::Full, SPOKEN));
    assert!(!offered.accepts(Form::Full, UNKNOWN));
    assert!(!offered.accepts(Form::Full, SILENT));
    assert!(offered.fit(louder()).is_some());
}

#[test]
fn a_plain_answer_is_accepted_only_when_it_speaks_and_acts_on_nothing() {
    let offered = offered();
    assert!(!offered.accepts(Form::Plain, LOUDER));
    assert!(!offered.accepts(Form::Plain, LOUDER_ALOUD));
    assert!(!offered.accepts(Form::Plain, UNKNOWN));
    assert!(!offered.accepts(Form::Plain, SILENT));
    assert!(offered.accepts(Form::Plain, SPOKEN));
    assert!(offered.accepts(Form::Plain, SPOKEN_WITH_NO_ACTIONS));
}

#[test]
fn no_action_fits_after_a_plain_answer_was_accepted() {
    let offered = offered();
    assert!(offered.accepts(Form::Plain, SPOKEN));
    assert!(offered.fit(louder()).is_none());
    assert!(!offered.accepts(Form::Plain, LOUDER_ALOUD));
    assert!(offered.fit(louder()).is_none());
    assert!(offered.accepts(Form::Full, SPOKEN));
    assert!(offered.fit(louder()).is_some());
}

#[test]
fn a_whole_answer_that_does_and_says_nothing_is_accepted_only_where_silence_is_allowed() {
    let overhearing = overhearing();
    assert!(overhearing.accepts(Form::Full, NOTHING));
    assert!(overhearing.accepts(Form::Full, SILENT));
    assert!(overhearing.accepts(Form::Full, NO_WORD));
    assert!(!overhearing.accepts(Form::Full, NOTHING_SO_FAR));
    assert!(!overhearing.accepts(Form::Full, UNKNOWN));
    assert!(!overhearing.accepts(Form::Full, "I will stay silent."));
    assert!(!overhearing.accepts(Form::Plain, NOTHING));
    assert!(!overhearing.accepts(Form::Plain, NO_WORD));
    assert!(overhearing.accepts(Form::Full, LOUDER));
    assert!(overhearing.accepts(Form::Full, SPOKEN));
    assert!(!offered().accepts(Form::Full, NOTHING));
    assert!(!offered().accepts(Form::Full, NO_WORD));
}

#[test]
fn a_reply_that_is_only_a_marked_name_counts_as_spoken() {
    let offered = offered();
    assert!(offered.accepts(Form::Full, ONLY_A_NAME));
    assert!(offered.accepts(Form::Plain, ONLY_A_NAME));
    assert!(!offered.accepts(Form::Full, A_NAME_SO_FAR));
    assert!(!offered.accepts(Form::Plain, A_NAME_SO_FAR));
    assert!(overhearing().accepts(Form::Full, ONLY_A_NAME));
}

#[tokio::test]
async fn the_say_data_of_a_marked_name_holds_the_text_and_how_to_speak_it() {
    let heard = streamed(NAMED, false).await;

    assert_eq!(
        heard.named(),
        [
            ("say", &json!({"text": "Включаю "})),
            ("say", &json!({"text": "Psychosis", "spoken": "Психосис"})),
            ("say", &json!({"text": "."})),
            ("done", &json!({})),
        ]
    );
}

#[tokio::test]
async fn an_overheard_phrase_answered_with_only_a_name_is_not_a_silence() {
    let heard = streamed(ONLY_A_NAME, true).await;

    assert_eq!(
        heard.named(),
        [
            ("say", &json!({"text": "Psychosis", "spoken": "Психосис"})),
            ("done", &json!({})),
        ]
    );
}
