use serde::Deserialize;

use super::*;

const EXAMPLE: &str = r#"
[providers.paid]
kind = "anthropic_compatible"
endpoint = "https://llm.example/v1/messages"
model = "example-model"
api_key = "env:EXAMPLE_KEY"
stream = true
max_output_tokens = 600
total_timeout_ms = 12000
input_usd_per_mtok = 2.0
output_usd_per_mtok = 8.0
body = { options = { effort = "low" } }

[providers.free]
kind = "openai_compatible"
endpoint = "https://llm.example/v1/chat/completions"
model = "example-model"
api_key = "env:EXAMPLE_KEY"
max_output_tokens = 900
total_timeout_ms = 14000

[providers.local]
kind = "worker"
subject = "ai.rpc.complete"
max_output_tokens = 400
total_timeout_ms = 15000

[ladders.chat]
steps = ["paid", "free", "local"]
hedge_after_ms = 2000
deadline_ms = 16000
breaker = { failures = 3, window_s = 30, open_s = 90 }

[ladders.chat.budget]
month_usd = 30.0
user_day_usd = 0.2

[budget]
bucket = "example_spend"

[feature]
ladder = "chat"
"#;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
    feature: Option<Feature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Feature {
    ladder: String,
}

fn parsed(text: &str) -> Result<(Settings, Host), ConfigError> {
    Settings::parse(text)
}

fn broken(text: &str) -> String {
    parsed(text).expect_err("the config is refused").to_string()
}

#[test]
fn the_three_kinds_a_ladder_and_the_host_section_are_read() {
    let (settings, host) = parsed(EXAMPLE).unwrap();
    assert_eq!(settings.providers.len(), 3);
    let paid = &settings.providers["paid"];
    assert_eq!(paid.kind, Kind::AnthropicCompatible);
    assert_eq!(
        (paid.input_usd_per_mtok, paid.output_usd_per_mtok),
        (2.0, 8.0)
    );
    assert_eq!(
        paid.api_key,
        Some(SecretRef::try_from("env:EXAMPLE_KEY".to_owned()).unwrap())
    );
    assert_eq!(paid.body.as_ref().unwrap()["options"]["effort"], "low");
    assert_eq!(paid.hedge, None);
    assert_eq!(settings.providers["free"].input_usd_per_mtok, 0.0);
    assert_eq!(settings.providers["free"].stream, None);
    assert_eq!(
        settings.providers["local"].subject.as_deref(),
        Some("ai.rpc.complete")
    );
    let chat = &settings.ladders["chat"];
    assert_eq!(chat.steps, ["paid", "free", "local"]);
    assert_eq!(chat.budget.month_usd, Some(30.0));
    assert_eq!(chat.budget.day_usd, None);
    assert_eq!(settings.budget.unwrap().bucket, "example_spend");
    assert_eq!(host.feature.unwrap().ladder, "chat");
}

#[test]
fn an_empty_file_means_no_ladders() {
    let (settings, host) = parsed("").unwrap();
    assert!(settings.providers.is_empty() && settings.ladders.is_empty());
    assert!(settings.budget.is_none() && host.feature.is_none());
}

#[test]
fn an_unknown_key_anywhere_stops_the_start() {
    assert!(broken(&EXAMPLE.replace("stream = true", "streem = true")).contains("streem"));
    assert!(broken(&EXAMPLE.replace("hedge_after_ms", "hedge_ms")).contains("hedge_ms"));
    assert!(broken(&EXAMPLE.replace("month_usd", "year_usd")).contains("year_usd"));
    assert!(broken(&format!("{EXAMPLE}\n[surprise]\nkey = 1\n")).contains("surprise"));
    assert!(
        broken(&EXAMPLE.replace("ladder = \"chat\"", "laddder = \"chat\"")).contains("laddder")
    );
}

#[test]
fn a_secret_written_in_the_file_is_refused() {
    let text = EXAMPLE.replacen("env:EXAMPLE_KEY", "sk-live-value", 1);
    let reason = broken(&text);
    assert!(reason.contains("env:NAME"), "{reason}");
    assert!(!reason.contains("sk-live-value"), "{reason}");
}

#[test]
fn a_ladder_names_only_known_providers_once() {
    let unknown =
        broken(&EXAMPLE.replace("[\"paid\", \"free\", \"local\"]", "[\"paid\", \"ghost\"]"));
    assert!(unknown.contains("unknown provider ghost"), "{unknown}");
    let twice = broken(&EXAMPLE.replace("[\"paid\", \"free\", \"local\"]", "[\"paid\", \"paid\"]"));
    assert!(twice.contains("twice"), "{twice}");
    let empty = broken(&EXAMPLE.replace("[\"paid\", \"free\", \"local\"]", "[]"));
    assert!(empty.contains("no steps"), "{empty}");
}

#[test]
fn each_kind_takes_only_its_own_keys() {
    let stray_subject = broken(&EXAMPLE.replace(
        "stream = true",
        "stream = true\nsubject = \"ai.rpc.complete\"",
    ));
    assert!(
        stray_subject.contains("only as a worker"),
        "{stray_subject}"
    );
    let stray_model = broken(&EXAMPLE.replace(
        "subject = \"ai.rpc.complete\"",
        "subject = \"ai.rpc.complete\"\nmodel = \"example-model\"",
    ));
    assert!(stray_model.contains("is a worker"), "{stray_model}");
    let long_answer =
        broken(&EXAMPLE.replace("max_output_tokens = 400", "max_output_tokens = 1025"));
    assert!(long_answer.contains("up to 1024"), "{long_answer}");
    assert!(
        parsed(&EXAMPLE.replace("max_output_tokens = 400", "max_output_tokens = 1024")).is_ok()
    );
    let no_endpoint =
        broken(&EXAMPLE.replacen("endpoint = \"https://llm.example/v1/messages\"\n", "", 1));
    assert!(no_endpoint.contains("needs endpoint"), "{no_endpoint}");
    let no_scheme = broken(&EXAMPLE.replacen("https://llm.example/v1/messages", "llm.example", 1));
    assert!(no_scheme.contains("http(s) URL"), "{no_scheme}");
}

#[test]
fn a_prefix_cache_and_cache_prices_are_read() {
    let text = EXAMPLE.replace(
        "stream = true",
        "prefix_cache = { type = \"ephemeral\", ttl = \"1h\" }\n\
         cache_read_usd_per_mtok = 0.2\ncache_write_usd_per_mtok = 2.5",
    );
    let (settings, _) = parsed(&text).unwrap();
    let paid = &settings.providers["paid"];
    assert_eq!(paid.prefix_cache.as_ref().unwrap()["ttl"], "1h");
    assert_eq!(paid.cache_read_usd_per_mtok, Some(0.2));
    assert_eq!(paid.cache_write_usd_per_mtok, Some(2.5));
    let free = &settings.providers["free"];
    assert!(free.prefix_cache.is_none() && free.cache_read_usd_per_mtok.is_none());
}

#[test]
fn a_prefix_cache_is_only_for_the_kind_that_marks_blocks() {
    let mark = "prefix_cache = { type = \"ephemeral\" }";
    let on_chat_completions = broken(&EXAMPLE.replace(
        "max_output_tokens = 900",
        &format!("max_output_tokens = 900\n{mark}"),
    ));
    assert!(
        on_chat_completions.contains("only as anthropic_compatible"),
        "{on_chat_completions}"
    );
    let on_a_worker = broken(&EXAMPLE.replace(
        "max_output_tokens = 400",
        &format!("max_output_tokens = 400\n{mark}"),
    ));
    assert!(on_a_worker.contains("is a worker"), "{on_a_worker}");
    let empty = broken(&EXAMPLE.replace("stream = true", "prefix_cache = {}"));
    assert!(empty.contains("empty prefix_cache"), "{empty}");
    let negative = broken(&EXAMPLE.replace("stream = true", "cache_read_usd_per_mtok = -0.1"));
    assert!(negative.contains("price"), "{negative}");
}

#[test]
fn caps_need_a_bucket_and_sane_numbers() {
    let no_bucket = broken(&EXAMPLE.replace("[budget]\nbucket = \"example_spend\"\n", ""));
    assert!(no_bucket.contains("[budget]"), "{no_bucket}");
    let negative = broken(&EXAMPLE.replace("month_usd = 30.0", "month_usd = -1.0"));
    assert!(negative.contains("spend cap"), "{negative}");
    let zero_window = broken(&EXAMPLE.replace("window_s = 30", "window_s = 0"));
    assert!(zero_window.contains("breaker"), "{zero_window}");
}

#[test]
fn static_headers_are_read_for_http_kinds_and_checked() {
    let with = |headers: &str| {
        EXAMPLE.replacen(
            "stream = true",
            &format!("stream = true\nheaders = {headers}"),
            1,
        )
    };
    let (settings, _) = parsed(&with("{ \"example-version\" = \"2001-01-01\" }")).unwrap();
    let headers = settings.providers["paid"].headers.as_ref().unwrap();
    assert_eq!(headers["example-version"], "2001-01-01");
    assert!(settings.providers["free"].headers.is_none());
    let bad_name = broken(&with("{ \"bad name\" = \"x\" }"));
    assert!(bad_name.contains("not a header name"), "{bad_name}");
    let bad_value = broken(&with("{ \"example-version\" = \"line\\nbreak\" }"));
    assert!(bad_value.contains("not a header value"), "{bad_value}");
    for reserved in ["x-api-key", "Authorization", "content-type"] {
        let reason = broken(&with(&format!("{{ \"{reserved}\" = \"x\" }}")));
        assert!(reason.contains("sets itself"), "{reason}");
    }
    let on_a_worker = broken(&EXAMPLE.replace(
        "max_output_tokens = 400",
        "max_output_tokens = 400\nheaders = { \"example-version\" = \"1\" }",
    ));
    assert!(on_a_worker.contains("is a worker"), "{on_a_worker}");
}

#[test]
fn a_step_can_be_kept_out_of_the_hedge() {
    let text = EXAMPLE.replace(
        "max_output_tokens = 400",
        "max_output_tokens = 400\nhedge = false",
    );
    let (settings, _) = parsed(&text).unwrap();
    assert_eq!(settings.providers["local"].hedge, Some(false));
}

#[test]
fn every_wait_has_an_upper_bound_and_a_step_ends_before_its_ladder() {
    let long_step =
        broken(&EXAMPLE.replace("total_timeout_ms = 12000", "total_timeout_ms = 600001"));
    assert!(
        long_step.contains("total_timeout_ms from 1 to 600000"),
        "{long_step}"
    );
    let long_deadline =
        broken(&EXAMPLE.replace("deadline_ms = 16000", "deadline_ms = 9223372036854775807"));
    assert!(
        long_deadline.contains("deadline_ms from 1"),
        "{long_deadline}"
    );
    let long_hedge = broken(&EXAMPLE.replace("hedge_after_ms = 2000", "hedge_after_ms = 900000"));
    assert!(long_hedge.contains("hedge_after_ms"), "{long_hedge}");
    let long_pause = broken(&EXAMPLE.replace("open_s = 90", "open_s = 86401"));
    assert!(
        long_pause.contains("open_s from 1 to 86400"),
        "{long_pause}"
    );
    let late_step =
        broken(&EXAMPLE.replace("total_timeout_ms = 14000", "total_timeout_ms = 16000"));
    assert!(
        late_step.contains("total_timeout_ms of free below its deadline_ms"),
        "{late_step}"
    );
    assert!(
        parsed(&EXAMPLE.replace("total_timeout_ms = 14000", "total_timeout_ms = 15999")).is_ok()
    );
}

#[test]
fn a_ladder_with_a_worker_step_takes_no_longer_system_text_than_the_worker() {
    let (settings, _) = parsed(EXAMPLE).unwrap();
    assert_eq!(settings.max_system_bytes("chat"), Some(48_000));
    let http_only = EXAMPLE.replace("[\"paid\", \"free\", \"local\"]", "[\"paid\", \"free\"]");
    let (settings, _) = parsed(&http_only).unwrap();
    assert_eq!(settings.max_system_bytes("chat"), None);
    assert_eq!(settings.max_system_bytes("ghost"), None);
}

#[test]
fn names_stay_inside_the_counter_key_alphabet() {
    let reason = broken(&EXAMPLE.replace("[ladders.chat", "[ladders.\"Chat Room\""));
    assert!(reason.contains("outside a-z"), "{reason}");
}

#[test]
fn dollars_become_whole_micro_dollars() {
    assert_eq!(micro_usd(0.3), 300_000);
    assert_eq!(micro_usd(7.25), 7_250_000);
    assert_eq!(micro_usd(0.0), 0);
}

#[test]
fn a_secret_reference_reads_the_environment_at_use() {
    let reference = SecretRef::try_from("env:LLM_LADDER_TEST_UNSET_KEY".to_owned()).unwrap();
    assert!(reference.resolve().is_none());
    assert!(SecretRef::try_from("env:".to_owned()).is_err());
}
