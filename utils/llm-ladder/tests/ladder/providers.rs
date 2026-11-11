use crate::ANSWER;
use crate::bench::{
    KEY, PIECES, Said, answer, answer_held_back, bench, http_provider, ladder, request, step,
};
use crate::vendor::{self, COMPLETIONS, MESSAGES, Reply, Vendor};
use llm_ladder::{Completion, Usage};
use serde_json::json;

const LAST_PIECE: usize = 5;

#[tokio::test]
async fn a_messages_stream_is_asked_in_its_wire_shape_and_read_piece_by_piece() {
    let vendor = Vendor::open().await;
    vendor.model(
        "stream-model",
        Reply::Gated(vendor::message_events(&PIECES, 1_000, 100), LAST_PIECE),
    );
    let config = http_provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "stream-model",
        "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\nheaders = { \"example-version\" = \"2001-01-01\" }\n\
         body = { max_tokens = 300, output_config = { effort = \"low\" } }",
    ) + &ladder("chat", &["paid"], 500);
    let bench = bench(&config, None);

    let early = format!("{}{}", PIECES[0], PIECES[1]);
    let answer = answer_held_back(&bench.ladders.get("chat").unwrap(), &vendor, &early).await;

    let outcome = answer.outcome.unwrap();
    assert_eq!(answer.document, ANSWER);
    assert_eq!(answer.deltas, 3);
    assert_eq!(outcome.provider, "paid");
    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 1_000,
            output_tokens: 100,
            ..Usage::default()
        }
    );
    assert_eq!(outcome.cost_micro_usd, 2_800);
    assert_eq!(
        bench.seen.steps(),
        [step("paid", "won", (1_000, 100), 2_800)]
    );

    let calls = vendor.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.path, MESSAGES);
    assert_eq!(call.header("x-api-key"), Some(KEY));
    assert_eq!(call.header("example-version"), Some("2001-01-01"));
    assert_eq!(call.header("anthropic-version"), None);
    assert_eq!(call.header("content-type"), Some("application/json"));
    assert_eq!(call.header("authorization"), None);
    assert_eq!(
        call.json(),
        json!({
            "model": "stream-model",
            "max_tokens": 300,
            "stream": true,
            "system": "Answer with one JSON object.\n\nSpeak as Loli.\n\nThe volume is 60.",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": "how are you"},
            ],
            "output_config": {
                "effort": "low",
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "say": {"type": "string"},
                            "mood": {"type": "string", "enum": ["calm", "glad"]},
                        },
                        "required": ["say", "mood"],
                        "additionalProperties": false,
                    },
                },
            },
        })
    );
    assert!(
        call.body.find("\"say\"").unwrap() < call.body.find("\"mood\"").unwrap(),
        "the schema keeps the order the caller gave: {}",
        call.body
    );
}

#[tokio::test]
async fn a_prefix_cache_marks_the_shared_blocks_and_cached_input_has_its_own_prices() {
    let vendor = Vendor::open().await;
    let start = json!({
        "input_tokens": 40,
        "cache_creation_input_tokens": 200,
        "cache_read_input_tokens": 3_000,
        "output_tokens": 1,
    });
    vendor.model(
        "stream-model",
        Reply::Events(vendor::message_events_counting(&PIECES, start, 70)),
    );
    let config = http_provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "stream-model",
        "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\n\
         cache_read_usd_per_mtok = 0.2\ncache_write_usd_per_mtok = 2.5\n\
         prefix_cache = { type = \"ephemeral\", ttl = \"1h\" }",
    ) + &ladder("chat", &["paid"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    let outcome = answer.outcome.unwrap();
    assert_eq!(answer.document, ANSWER);
    let usage = Usage {
        input_tokens: 40,
        cache_read_tokens: 3_000,
        cache_write_tokens: 200,
        output_tokens: 70,
    };
    assert_eq!(outcome.usage, usage);
    assert_eq!(outcome.cost_micro_usd, 80 + 600 + 500 + 560);
    assert_eq!(bench.seen.steps()[0].usage, usage);
    assert_eq!(bench.seen.steps()[0].cost_micro_usd, 1_740);
    let body = vendor.calls()[0].json();
    assert_eq!(
        body["system"],
        json!([
            {
                "type": "text",
                "text": "Answer with one JSON object.",
                "cache_control": {"type": "ephemeral", "ttl": "1h"},
            },
            {
                "type": "text",
                "text": "\n\nSpeak as Loli.",
                "cache_control": {"type": "ephemeral", "ttl": "1h"},
            },
            {"type": "text", "text": "\n\nThe volume is 60."},
        ])
    );
    assert_eq!(
        body["messages"][0],
        json!({"role": "user", "content": "hi"})
    );
}

#[tokio::test]
async fn cached_prompt_tokens_of_a_chat_completion_are_charged_as_cache_reads() {
    let vendor = Vendor::open().await;
    let completion = json!({
        "choices": [{"index": 0, "message": {"role": "assistant", "content": ANSWER}}],
        "usage": {
            "prompt_tokens": 3_200,
            "completion_tokens": 70,
            "prompt_tokens_details": {"cached_tokens": 3_000},
        },
    });
    vendor.model("whole-model", Reply::Whole(0, completion));
    let config = http_provider(
        "proxy",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "whole-model",
        "stream = false\ninput_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\n\
         cache_read_usd_per_mtok = 0.2",
    ) + &ladder("chat", &["proxy"], 500);
    let bench = bench(&config, None);

    let outcome = answer(&bench.ladders.get("chat").unwrap())
        .await
        .outcome
        .unwrap();

    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 200,
            cache_read_tokens: 3_000,
            cache_write_tokens: 0,
            output_tokens: 70,
        }
    );
    assert_eq!(outcome.cost_micro_usd, 400 + 600 + 560);
}

#[tokio::test]
async fn a_messages_one_shot_answer_comes_as_a_single_delta() {
    let vendor = Vendor::open().await;
    vendor.model(
        "whole-model",
        Reply::Whole(50, vendor::whole_message(ANSWER, 1_000, 100)),
    );
    let config = http_provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "whole-model",
        "stream = false\ninput_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0",
    ) + &ladder("chat", &["paid"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!((answer.document.as_str(), answer.deltas), (ANSWER, 1));
    assert_eq!(answer.outcome.unwrap().cost_micro_usd, 2_800);
    assert!(vendor.calls()[0].json().get("stream").is_none());
}

#[tokio::test]
async fn a_chat_completions_stream_is_asked_in_its_wire_shape_and_reasoning_is_dropped() {
    let vendor = Vendor::open().await;
    let fenced = ["```json\n{\"say\":", "\"hello ", "there\"}\n```"];
    vendor.model(
        "stream-model",
        Reply::Gated(vendor::completion_chunks(&fenced, Some((90, 120))), 3),
    );
    let config = http_provider(
        "proxy",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "stream-model",
        "body = { temperature = 0.2 }",
    ) + &ladder("chat", &["proxy"], 500);
    let bench = bench(&config, None);

    let early = format!("{}{}", PIECES[0], PIECES[1]);
    let answer = answer_held_back(&bench.ladders.get("chat").unwrap(), &vendor, &early).await;

    let outcome = answer.outcome.unwrap();
    assert_eq!(answer.document, ANSWER);
    assert_eq!(answer.deltas, 3);
    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 90,
            output_tokens: 120,
            ..Usage::default()
        }
    );
    assert_eq!(
        outcome.cost_micro_usd, 0,
        "a provider without a price costs nothing"
    );

    let call = &vendor.calls()[0];
    assert_eq!(call.path, COMPLETIONS);
    assert_eq!(call.header("authorization"), Some("Bearer llm-ladder"));
    assert_eq!(call.header("x-api-key"), None);
    let body = call.json();
    assert_eq!(body["model"], "stream-model");
    assert_eq!(body["max_tokens"], 600);
    assert_eq!(body["temperature"], 0.2);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "Answer with one JSON object.\n\nSpeak as Loli.\n\nThe volume is 60."},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
            {"role": "user", "content": "how are you"},
        ])
    );
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["required"],
        json!(["say", "mood"])
    );
}

#[tokio::test]
async fn a_chat_completions_one_shot_answer_without_usage_still_reads() {
    let vendor = Vendor::open().await;
    let fenced = format!("Sure!\n```json\n{ANSWER}\n```\nAnything else?");
    vendor.model(
        "whole-model",
        Reply::Whole(50, vendor::whole_completion(&fenced, None)),
    );
    let config = http_provider(
        "proxy",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "whole-model",
        "stream = false",
    ) + &ladder("chat", &["proxy"], 500);
    let bench = bench(&config, None);

    let completion: Completion<Said> = bench
        .ladders
        .get("chat")
        .unwrap()
        .complete(request())
        .await
        .unwrap();

    assert_eq!(completion.value.say, "hello there");
    assert_eq!(completion.outcome.usage, Usage::default());
    let body = vendor.calls()[0].json();
    assert!(body.get("stream").is_none() && body.get("stream_options").is_none());
}
