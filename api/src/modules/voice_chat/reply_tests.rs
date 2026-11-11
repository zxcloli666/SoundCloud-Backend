use serde_json::{Value, json};

use super::*;
use crate::modules::voice_chat::testing::tools;
use crate::modules::voice_chat::tools::ToolSpec;

const LOUDER: &str = "{\"actions\":[{\"name\":\"volume_set\",\"args\":[{\"name\":\"level\",\"value\":90}]}],\"say\":\"\"}";
const LOUDER_ALOUD: &str = "{\"actions\":[{\"name\":\"volume_set\",\"args\":[{\"name\":\"level\",\"value\":90}]}],\"say\":\"Громче.\"}";
const UNKNOWN: &str = "{\"actions\":[{\"name\":\"erase_library\",\"args\":[]}],\"say\":\"\"}";
const SPOKEN: &str = "{\"say\":\"Привет.\"}";
const SPOKEN_WITH_NO_ACTIONS: &str = "{\"actions\":[],\"say\":\"Привет.\"}";
const SILENT: &str = "{\"actions\":[],\"say\":\" \"}";

fn offered() -> Arc<Offered> {
    let specs: Vec<ToolSpec> = serde_json::from_value(tools()).expect("a tool list");
    Offered::new(Tools::read(specs, 8).expect("a tool set"))
}

fn louder() -> Value {
    json!({"name": "volume_set", "args": [{"name": "level", "value": 90}]})
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
