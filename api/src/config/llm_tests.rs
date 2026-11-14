use super::*;

const EXAMPLE: &str = r#"
[providers.paid]
kind = "anthropic_compatible"
endpoint = "https://llm.example/v1/messages"
model = "example-model"
api_key = "env:EXAMPLE_KEY"
max_output_tokens = 600
total_timeout_ms = 12000

[ladders.chat]
steps = ["paid"]
hedge_after_ms = 2000
deadline_ms = 16000
breaker = { failures = 3, window_s = 30, open_s = 90 }

[voice_chat]
ladder = "chat"
requests_per_minute = 12
max_text_chars = 400
max_history = 10
max_notes = 20
max_tools = 48
max_tool_rounds = 2
max_system_bytes = 24000
instructions = "Keep it short."

[voice_chat.personas.default]
prompt = "A friendly voice."

[voice_chat.personas.loli]
prompt = "A cheerful voice."
"#;

const PHRASES: &str = r#"
[voice_speak.phrases]
endpoint = "https://s3.example"
bucket = "voice-phrases"
access_key = "env:CARGO_PKG_NAME"
secret_key = "env:CARGO_PKG_NAME"
"#;

fn refused(text: &str) -> String {
    LlmCfg::parse(text)
        .expect_err("the config is refused")
        .to_string()
}

#[test]
fn the_chat_section_is_read_next_to_the_ladders() {
    let config = LlmCfg::parse(EXAMPLE).unwrap();
    let chat = config.voice_chat.unwrap();
    assert_eq!(chat.ladder, "chat");
    assert_eq!(chat.requests_per_minute, 12);
    assert_eq!(
        (chat.max_text_chars, chat.max_history, chat.max_notes),
        (400, 10, 20)
    );
    assert_eq!((chat.max_tools, chat.max_tool_rounds), (48, 2));
    assert_eq!(chat.max_system_bytes, 24_000);
    assert_eq!(chat.instructions, "Keep it short.");
    assert!(config.ladders.ladders.contains_key("chat"));
}

#[test]
fn an_unknown_persona_speaks_as_the_default_one() {
    let chat = LlmCfg::parse(EXAMPLE).unwrap().voice_chat.unwrap();
    assert_eq!(chat.persona("loli"), "A cheerful voice.");
    assert_eq!(chat.persona("nobody"), "A friendly voice.");
    assert_eq!(chat.persona(""), "A friendly voice.");
}

#[test]
fn a_file_without_the_chat_section_has_ladders_and_no_chat() {
    let cut = EXAMPLE.split("[voice_chat]").next().unwrap();
    let config = LlmCfg::parse(cut).unwrap();
    assert!(config.voice_chat.is_none());
    assert_eq!(config.ladders.ladders.len(), 1);
    let empty = LlmCfg::parse("").unwrap();
    assert!(empty.voice_chat.is_none() && empty.ladders.ladders.is_empty());
}

#[test]
fn a_chat_section_that_cannot_work_stops_the_start() {
    let no_ladder = refused(&EXAMPLE.replace("ladder = \"chat\"", "ladder = \"talk\""));
    assert!(no_ladder.contains("unknown ladder talk"), "{no_ladder}");
    let no_default = refused(&EXAMPLE.replace("personas.default", "personas.other"));
    assert!(no_default.contains("personas.default"), "{no_default}");
    let blank = refused(&EXAMPLE.replace("A cheerful voice.", "  "));
    assert!(
        blank.contains("persona loli has an empty prompt"),
        "{blank}"
    );
    let no_rate = refused(&EXAMPLE.replace("requests_per_minute = 12", "requests_per_minute = 0"));
    assert!(no_rate.contains("requests_per_minute"), "{no_rate}");
    let typo = refused(&EXAMPLE.replace("max_notes", "max_note"));
    assert!(typo.contains("max_note"), "{typo}");
    let stray = refused(&format!(
        "{EXAMPLE}\n[voice_chat.personas.loli.extra]\nkey = 1\n"
    ));
    assert!(stray.contains("extra"), "{stray}");
    let unknown_section = refused(&format!("{EXAMPLE}\n[speech]\nkey = 1\n"));
    assert!(unknown_section.contains("speech"), "{unknown_section}");
}

#[test]
fn the_system_text_cap_must_fit_a_worker_step_of_the_ladder() {
    let worker = "[providers.local]\nkind = \"worker\"\nsubject = \"ai.rpc.complete\"\n\
                  max_output_tokens = 400\ntotal_timeout_ms = 9000\n";
    let with_worker = EXAMPLE.replace("steps = [\"paid\"]", "steps = [\"paid\", \"local\"]");
    let config = format!("{worker}{with_worker}");
    let fits = LlmCfg::parse(&config).unwrap().voice_chat.unwrap();
    assert_eq!(fits.max_system_bytes, 24_000);
    let at_limit = config.replace("max_system_bytes = 24000", "max_system_bytes = 48000");
    assert!(LlmCfg::parse(&at_limit).is_ok());
    let over = refused(&config.replace("max_system_bytes = 24000", "max_system_bytes = 48001"));
    assert!(over.contains("max_system_bytes is above 48000"), "{over}");
    let http_only = EXAMPLE.replace("max_system_bytes = 24000", "max_system_bytes = 200000");
    assert!(LlmCfg::parse(&http_only).is_ok());
    let zero = refused(&EXAMPLE.replace("max_system_bytes = 24000", "max_system_bytes = 0"));
    assert!(
        zero.contains("max_system_bytes must be above zero"),
        "{zero}"
    );
    let missing = refused(&EXAMPLE.replace("max_system_bytes = 24000\n", ""));
    assert!(missing.contains("max_system_bytes"), "{missing}");
}

#[test]
fn the_chat_rate_from_the_file_becomes_the_per_user_admission_limit() {
    let chat = LlmCfg::parse(EXAMPLE).unwrap().voice_chat.unwrap();
    let closed = crate::config::AdmissionCfg::from_env();
    assert_eq!(
        (closed.voice_chat.per_client, closed.voice_chat.global),
        (0, 0)
    );
    let open = closed.with_voice_chat(Some(&chat));
    assert_eq!(
        (open.voice_chat.per_client, open.voice_chat.global),
        (12, u32::MAX)
    );
}

fn speak_file(section: &str) -> String {
    format!("{EXAMPLE}\n[voice_speak]\n{section}\n{PHRASES}")
}

fn phrases_file(phrases: &str) -> String {
    format!("{EXAMPLE}\n[voice_speak]\nrequests_per_minute = 5\n\n[voice_speak.phrases]\n{phrases}")
}

#[test]
fn the_speak_section_names_its_rate_and_keeps_four_lines_in_flight_unless_told() {
    let parsed = |section: &str| {
        LlmCfg::parse(&speak_file(section))
            .unwrap()
            .voice_speak
            .unwrap()
    };

    let default = parsed("requests_per_minute = 20");
    let wider = parsed("requests_per_minute = 20\nmax_in_flight = 6");

    assert_eq!(
        (default.requests_per_minute, default.max_in_flight),
        (20, 4)
    );
    assert_eq!(wider.max_in_flight, 6);
    assert!(LlmCfg::parse(EXAMPLE).unwrap().voice_speak.is_none());
    let silent = refused(&speak_file("requests_per_minute = 0"));
    assert!(
        silent.contains("[voice_speak] requests_per_minute must be above zero"),
        "{silent}"
    );
    let stuck = refused(&speak_file("requests_per_minute = 5\nmax_in_flight = 0"));
    assert!(
        stuck.contains("max_in_flight must be above zero"),
        "{stuck}"
    );
    assert!(refused(&speak_file("max_in_flight = 2")).contains("requests_per_minute"));
}

#[test]
fn the_speak_section_reads_the_phrase_store_and_its_keys_from_the_environment() {
    let speak = LlmCfg::parse(&speak_file("requests_per_minute = 20"))
        .unwrap()
        .voice_speak
        .unwrap();
    let phrases = speak.phrases.unwrap();
    let keys = phrases.keys().unwrap();

    assert_eq!(phrases.endpoint, "https://s3.example");
    assert_eq!(phrases.bucket, "voice-phrases");
    assert_eq!(keys.access_key.expose(), env!("CARGO_PKG_NAME"));
    assert_eq!(keys.secret_key.expose(), env!("CARGO_PKG_NAME"));
}

#[test]
fn a_speak_section_without_a_phrase_table_runs_without_a_cache() {
    let speak = LlmCfg::parse(&format!(
        "{EXAMPLE}\n[voice_speak]\nrequests_per_minute = 5\n"
    ))
    .unwrap()
    .voice_speak
    .unwrap();

    assert!(speak.phrases.is_none());
    assert_eq!(speak.requests_per_minute, 5);
}

#[test]
fn a_phrase_table_without_every_field_is_refused() {
    let fields = [
        ("endpoint", "\"https://s3.example\""),
        ("bucket", "\"voice-phrases\""),
        ("access_key", "\"env:CARGO_PKG_NAME\""),
        ("secret_key", "\"env:CARGO_PKG_NAME\""),
    ];
    let table = |field: &str, value: &str| {
        fields
            .iter()
            .map(|(name, kept)| {
                let value = if *name == field { value } else { kept };
                format!("{name} = {value}\n")
            })
            .collect::<String>()
    };
    let without = |field: &str| {
        fields
            .iter()
            .filter(|(name, _)| *name != field)
            .map(|(name, value)| format!("{name} = {value}\n"))
            .collect::<String>()
    };

    assert!(refused(&phrases_file("")).contains("endpoint"));
    for (field, _) in fields {
        let lacking = refused(&phrases_file(&without(field)));
        assert!(lacking.contains(field), "{lacking}");
    }
    let refusals = [
        (
            table("endpoint", "\" \""),
            "[voice_speak] phrases.endpoint is empty",
        ),
        (
            table("endpoint", "\"s3.example\""),
            "[voice_speak] phrases.endpoint is not an http or https URL",
        ),
        (
            table("bucket", "\"\""),
            "[voice_speak] phrases.bucket is empty",
        ),
        (
            table("access_key", "\"env:VOICE_SPEAK_TEST_UNSET_KEY\""),
            "[voice_speak] phrases.access_key names a variable that is unset or empty",
        ),
        (
            table("secret_key", "\"env:VOICE_SPEAK_TEST_UNSET_KEY\""),
            "[voice_speak] phrases.secret_key names a variable that is unset or empty",
        ),
        (
            table("secret_key", "\"plain-secret\""),
            "a secret is written as env:NAME",
        ),
    ];
    for (phrases, reason) in refusals {
        let refusal = refused(&phrases_file(&phrases));
        assert!(refusal.contains(reason), "{refusal}");
        assert!(!refusal.contains("plain-secret"), "{refusal}");
    }
}

#[test]
fn the_speak_rate_from_the_file_becomes_the_per_user_admission_limit() {
    let speak = LlmCfg::parse(&speak_file("requests_per_minute = 20"))
        .unwrap()
        .voice_speak
        .unwrap();
    let closed = crate::config::AdmissionCfg::from_env();
    assert_eq!(
        (closed.voice_speak.per_client, closed.voice_speak.global),
        (0, 0)
    );
    let open = closed.with_voice_speak(Some(&speak));
    assert_eq!(
        (open.voice_speak.per_client, open.voice_speak.global),
        (20, u32::MAX)
    );
}

#[test]
fn the_example_file_in_the_repository_is_a_valid_config() {
    let example = include_str!("../../llm.toml.example")
        .replace("env:EXAMPLE_PHRASES_ACCESS_KEY", "env:CARGO_PKG_NAME")
        .replace("env:EXAMPLE_PHRASES_SECRET_KEY", "env:CARGO_PKG_NAME");
    let example = LlmCfg::parse(&example).unwrap();
    let chat = example.voice_chat.unwrap();
    assert!(example.ladders.ladders.contains_key(&chat.ladder));
    assert!(chat.personas.contains_key(DEFAULT_PERSONA));
    assert!(example.ladders.budget.is_some());
    assert!(example.voice_speak.is_some());
}
