use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use futures::StreamExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::super::router_search_tests::{soundcloud_hits, state_with};
use super::{cap, session_for, tuned};
use crate::bus::nats::NatsService;
use crate::config::{AppConfig, LlmCfg};
use crate::modules::voice_chat::VoiceChat;
use crate::modules::voice_chat::testing::{CHAT_SECTION, Heard, hear, lane, nats_url, tools};
use vendor::{COMPLETIONS, End, MESSAGES, Reply, Vendor};

#[path = "router_voice_chat_cache_tests.rs"]
mod cache;
#[path = "router_voice_chat_test_vendor.rs"]
mod vendor;

const GAP_MS: u64 = 150;
const LOUDER: [&str; 4] = [
    "{\"actions\":[{\"name\":\"volume_set\",",
    "\"args\":[{\"name\":\"level\",\"value\":60}]}],",
    "\"say\":\"Сделала ",
    "погромче.\"}",
];
const ANSWER_COST_MICRO_USD: &str = "2800";

fn provider(id: &str, kind: &str, url: &str, model: &str, more: &str) -> String {
    format!(
        "[providers.{id}]\nkind = \"{kind}\"\nendpoint = \"{url}\"\nmodel = \"{model}\"\n\
         api_key = \"env:CARGO_PKG_NAME\"\nmax_output_tokens = 600\ntotal_timeout_ms = 4000\n{more}\n\n"
    )
}

fn ladder(steps: &[&str], more: &str) -> String {
    let steps: Vec<String> = steps.iter().map(|step| format!("\"{step}\"")).collect();
    format!(
        "[ladders.chat]\nsteps = [{}]\nhedge_after_ms = 3000\ndeadline_ms = 8000\n\
         breaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n{more}\n",
        steps.join(", ")
    )
}

fn one_step(vendor: &Vendor, model: &str) -> String {
    provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        model,
        "",
    ) + &ladder(&["paid"], "")
        + CHAT_SECTION
}

fn fresh_user() -> String {
    (Uuid::now_v7().as_u128() % 1_000_000_000_000_000).to_string()
}

fn turn() -> Value {
    json!({
        "text": "сделай погромче",
        "persona": "loli",
        "language": "ru",
        "context": {"volume": 40},
        "history": [{"role": "user", "text": "привет"}, {"role": "assistant", "text": "Привет!"}],
        "notes": ["любит джаз"],
        "tools": tools(),
    })
}

async fn chat_app(
    pool: &PgPool,
    config: AppConfig,
    star: bool,
    llm: &str,
    nats: Option<&Arc<NatsService>>,
) -> anyhow::Result<axum::Router> {
    let mut state = state_with(pool, soundcloud_hits(), config).await?;
    if !star {
        state.subscriptions =
            crate::modules::subscriptions::SubscriptionsService::new(pool.clone(), false);
    }
    state.voice_chat = VoiceChat::start(&LlmCfg::parse(llm)?, nats)?;
    Ok(crate::router::build(state))
}

async fn starred(pool: &PgPool, llm: &str) -> anyhow::Result<axum::Router> {
    chat_app(pool, tuned(|_| {}), true, llm, None).await
}

async fn open(
    app: &axum::Router,
    session: Option<Uuid>,
    body: &Value,
    more: &[(&str, &str)],
) -> anyhow::Result<Response> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/voice/chat")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(session) = session {
        request = request.header("x-session-id", session.to_string());
    }
    for (name, value) in more {
        request = request.header(*name, *value);
    }
    Ok(app
        .clone()
        .oneshot(request.body(Body::from(body.to_string()))?)
        .await?)
}

async fn ask(app: &axum::Router, session: Uuid, body: &Value) -> anyhow::Result<Heard> {
    hear(open(app, Some(session), body, &[]).await?).await
}

async fn all_connections_idle(pool: &PgPool) -> bool {
    let limit = Instant::now() + Duration::from_secs(3);
    while pool.num_idle() as u32 != pool.size() {
        if Instant::now() > limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_turn_streams_its_action_then_the_speech_piece_by_piece(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&LOUDER, GAP_MS, 1_000, 100), End::Closes),
    );
    let app = starred(&pool, &one_step(&vendor, "paid-model")).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let response = open(
        &app,
        Some(session),
        &turn(),
        &[("accept-encoding", "gzip, br")],
    )
    .await?;
    let heard = hear(response).await?;

    assert_eq!(heard.status, StatusCode::OK);
    assert_eq!(heard.header("content-type"), Some("text/event-stream"));
    assert_eq!(
        heard.header("content-encoding"),
        None,
        "an event stream is never compressed"
    );
    assert_eq!(heard.header("content-length"), None);
    assert_eq!(
        heard.header("cache-control"),
        Some("private, no-store, max-age=0")
    );
    assert_eq!(
        heard.named(),
        [
            (
                "action",
                &json!({"name": "volume_set", "args": {"level": 60}})
            ),
            ("say", &json!({"text": "Сделала "})),
            ("say", &json!({"text": "погромче."})),
            ("done", &json!({})),
        ]
    );
    let (first, last) = (heard.events[0].0, heard.events[3].0);
    assert!(
        last - first >= Duration::from_millis(2 * GAP_MS - 50),
        "events leave as the provider speaks, not together at the end: {first:?} .. {last:?}"
    );

    let calls = vendor.calls();
    assert_eq!(calls.len(), 1);
    let asked = &calls[0];
    assert_eq!(asked.headers.get("x-api-key").unwrap(), "api");
    let system = asked.body["system"].as_str().unwrap();
    for part in [
        "A cheerful voice named Loli.",
        "Keep it short.",
        "in the language with the code \"ru\"",
        "- volume_set(level: integer 0..100): Set the playback volume.",
        "- search_tracks(query: string): Find tracks by name. (returns a result)",
        "<notes>\n[\"любит джаз\"]\n</notes>",
        "<app_state>\n{\"volume\":40}\n</app_state>",
        "Whose words count.",
    ] {
        assert!(system.contains(part), "{part} is missing in\n{system}");
    }
    assert_eq!(
        asked.body["messages"],
        json!([
            {"role": "user", "content": "привет"},
            {"role": "assistant", "content": "{\"actions\":[],\"say\":\"Привет!\"}"},
            {"role": "user", "content": "сделай погромче"},
        ])
    );
    let schema = &asked.body["output_config"]["format"]["schema"];
    assert_eq!(
        schema["properties"]["actions"]["items"]["properties"]["name"]["enum"],
        json!(["volume_set", "search_tracks"])
    );
    assert_eq!(asked.body["stream"], true);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_star_answer_is_the_same_on_a_reserve_host(pool: PgPool) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&LOUDER, 0, 1_000, 100), End::Closes),
    );
    let llm = one_step(&vendor, "paid-model");
    let reserve = || tuned(|config| config.premium_reserve = true);
    let plain_host = chat_app(&pool, tuned(|_| {}), false, &llm, None).await?;
    let reserve_host = chat_app(&pool, reserve(), false, &llm, None).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let plain = ask(&plain_host, session, &turn()).await?;
    let reserved = ask(&reserve_host, session, &turn()).await?;

    assert_eq!(plain.status, StatusCode::FORBIDDEN);
    assert_eq!(plain.body["code"], "star_required");
    assert_eq!(plain.body["statusCode"], 403);
    assert_eq!(
        (reserved.status, &reserved.body),
        (plain.status, &plain.body)
    );
    assert!(
        vendor.calls().is_empty(),
        "no provider is called for a listener without Star"
    );

    let starred_reserve = chat_app(&pool, reserve(), true, &llm, None).await?;
    let heard = ask(&starred_reserve, session, &turn()).await?;
    assert_eq!(heard.status, StatusCode::OK);
    assert_eq!(heard.named().last().unwrap().0, "done");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_chat_that_cannot_start_answers_503(pool: PgPool) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let not_configured = starred(&pool, "").await?;
    let heard = ask(&not_configured, session, &turn()).await?;
    assert_eq!(heard.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(heard.body["code"], "chat_unavailable");
    assert_eq!(heard.header("retry-after"), Some("30"));

    let keyless = one_step(&vendor, "paid-model")
        .replace("env:CARGO_PKG_NAME", "env:VOICE_CHAT_TEST_UNSET_KEY");
    let no_open_step = starred(&pool, &keyless).await?;
    let heard = ask(&no_open_step, session, &turn()).await?;
    assert_eq!(heard.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(heard.body["code"], "chat_unavailable");
    assert!(vendor.calls().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_per_user_limit_answers_429_and_spares_other_users(pool: PgPool) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&LOUDER, 0, 1_000, 100), End::Closes),
    );
    let limited = tuned(|config| config.admission.voice_chat = cap(2));
    let app = chat_app(&pool, limited, true, &one_step(&vendor, "paid-model"), None).await?;
    let chatty = session_for(&pool, &fresh_user()).await?;
    let quiet = session_for(&pool, &fresh_user()).await?;

    for _ in 0..2 {
        assert_eq!(ask(&app, chatty, &turn()).await?.status, StatusCode::OK);
    }
    let refused = ask(&app, chatty, &turn()).await?;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(refused.body["code"], "voice_chat_limited");
    let wait: u64 = refused.header("retry-after").unwrap().parse()?;
    assert!((1..=60).contains(&wait), "the limit is per minute: {wait}");
    assert_eq!(ask(&app, quiet, &turn()).await?.status, StatusCode::OK);
    assert_eq!(vendor.calls().len(), 3);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_request_over_a_cap_or_without_a_session_reaches_no_provider(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let app = starred(&pool, &one_step(&vendor, "paid-model")).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let mut long = turn();
    long["text"] = json!("я".repeat(41));
    let heard = ask(&app, session, &long).await?;
    assert_eq!(heard.status, StatusCode::BAD_REQUEST);
    assert_eq!(heard.body["code"], "voice_chat_invalid");
    assert_eq!(heard.body["message"], "text is longer than 40 characters");

    let mut odd_tool = turn();
    odd_tool["tools"][0]["args"]["properties"]["level"] = json!({"type": "object"});
    let heard = ask(&app, session, &odd_tool).await?;
    assert_eq!(heard.status, StatusCode::BAD_REQUEST);
    assert!(
        heard.body["message"]
            .as_str()
            .unwrap()
            .contains("tool volume_set")
    );

    let mut shapeless = turn();
    shapeless["history"] = json!(["сделай погромче"]);
    let heard = ask(&app, session, &shapeless).await?;
    assert_eq!(heard.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(heard.body["code"], "voice_chat_invalid");
    assert_eq!(
        heard.body["message"],
        "the body does not have the shape of a chat request"
    );

    let anonymous = hear(open(&app, None, &turn(), &[]).await?).await?;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(vendor.calls().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_ladder_that_breaks_after_the_start_ends_the_stream_with_an_error(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model("down-model", Reply::Status(529));
    vendor.model(
        "cut-model",
        Reply::Stream(
            vendor::message(&LOUDER[..3], 0, 1_000, 100)[..4].to_vec(),
            End::Breaks,
        ),
    );
    vendor.model(
        "stalling-model",
        Reply::Stream(
            vendor::message(&LOUDER[..2], 0, 1_000, 100)[..3].to_vec(),
            End::Stalls,
        ),
    );
    let nothing = ["{\"actions\":[{\"name\":\"open_door\",\"args\":[]}],\"say\":\" \"}"];
    vendor.model(
        "mute-model",
        Reply::Stream(vendor::message(&nothing, 0, 1_000, 100), End::Closes),
    );
    let session = session_for(&pool, &fresh_user()).await?;

    let mute = starred(&pool, &one_step(&vendor, "mute-model")).await?;
    let heard = ask(&mute, session, &turn()).await?;
    assert_eq!(
        heard.named(),
        [
            ("say", &json!({"text": " "})),
            ("error", &json!({"code": "provider_failed"}))
        ],
        "an answer with nothing to do and nothing to say is a failure"
    );

    let down = starred(&pool, &one_step(&vendor, "down-model")).await?;
    let heard = ask(&down, session, &turn()).await?;
    assert_eq!(heard.status, StatusCode::OK);
    assert_eq!(
        heard.named(),
        [("error", &json!({"code": "provider_failed"}))]
    );

    let cut = starred(&pool, &one_step(&vendor, "cut-model")).await?;
    let heard = ask(&cut, session, &turn()).await?;
    assert_eq!(
        heard.named(),
        [
            (
                "action",
                &json!({"name": "volume_set", "args": {"level": 60}})
            ),
            ("say", &json!({"text": "Сделала "})),
            ("error", &json!({"code": "provider_failed"})),
        ]
    );

    let hurried =
        one_step(&vendor, "stalling-model").replace("deadline_ms = 8000", "deadline_ms = 2000");
    let stalling = starred(&pool, &hurried).await?;
    let heard = ask(&stalling, session, &turn()).await?;
    assert_eq!(
        heard.named(),
        [
            (
                "action",
                &json!({"name": "volume_set", "args": {"level": 60}})
            ),
            ("error", &json!({"code": "deadline"})),
        ]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_client_that_leaves_cancels_the_provider_call_and_the_stream_holds_no_connection(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model(
        "stalling-model",
        Reply::Stream(
            vendor::message(&LOUDER[..2], 0, 1_000, 100)[..3].to_vec(),
            End::Stalls,
        ),
    );
    let app = starred(&pool, &one_step(&vendor, "stalling-model")).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let response = open(&app, Some(session), &turn(), &[]).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let mut frames = response.into_body().into_data_stream();
    let first = frames.next().await.expect("the action arrives")?;
    assert!(String::from_utf8_lossy(&first).starts_with("event: action\n"));
    assert!(
        all_connections_idle(&pool).await,
        "an open chat stream keeps no database connection"
    );
    drop(frames);

    assert!(
        vendor.hung_up(1).await,
        "the provider call is closed when the client leaves"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn actions_that_do_not_fit_a_tool_are_left_out(pool: PgPool) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let answer = json!({
        "actions": [
            {"name": "open_door", "args": []},
            {"name": "volume_set", "args": [{"name": "level", "value": "loud"}]},
            {"name": "volume_set", "args": {"level": "70"}},
        ],
        "say": "Готово.",
    })
    .to_string();
    let fenced = format!("<think>the user wants it louder</think>\n```json\n{answer}\n```");
    vendor.model(
        "free-model",
        Reply::Stream(vendor::completion(&[fenced.as_str()], 0), End::Closes),
    );
    let llm = provider(
        "free",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "free-model",
        "",
    ) + &ladder(&["free"], "")
        + CHAT_SECTION;
    let app = starred(&pool, &llm).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let heard = ask(&app, session, &turn()).await?;

    assert_eq!(
        heard.named(),
        [
            (
                "action",
                &json!({"name": "volume_set", "args": {"level": 70}})
            ),
            ("say", &json!({"text": "Готово."})),
            ("done", &json!({})),
        ]
    );
    let asked = &vendor.calls()[0].body;
    assert_eq!(asked["messages"][0]["role"], "system");
    assert_eq!(asked["response_format"]["json_schema"]["strict"], true);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn tool_results_travel_in_the_last_message_and_end_the_tool_rounds(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let spoken = ["{\"actions\":[],\"say\":\"Нашла «Ночь».\"}"];
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&spoken, 0, 1_000, 100), End::Closes),
    );
    let app = starred(&pool, &one_step(&vendor, "paid-model")).await?;
    let session = session_for(&pool, &fresh_user()).await?;
    let result =
        json!({"name": "search_tracks", "args": {"query": "ночь"}, "result": {"tracks": ["Ночь"]}});

    let mut first_round = turn();
    first_round["results"] = json!([result]);
    let heard = ask(&app, session, &first_round).await?;
    assert_eq!(
        heard.named(),
        [
            ("say", &json!({"text": "Нашла «Ночь»."})),
            ("done", &json!({}))
        ]
    );
    let mut last_round = turn();
    last_round["results"] = json!([result, result]);
    assert_eq!(
        ask(&app, session, &last_round).await?.status,
        StatusCode::OK
    );

    let calls = vendor.calls();
    let block = "сделай погромче\n\n<tool_results>\n[{\"name\":\"search_tracks\",\"args\":{\"query\":\"ночь\"},\"result\":{\"tracks\":[\"Ночь\"]}}]\n</tool_results>";
    assert_eq!(calls[0].body["messages"][2]["content"], block);
    let offered = |call: &vendor::Call| {
        let schema = &call.body["output_config"]["format"]["schema"];
        schema["properties"]["actions"]["items"]["properties"]["name"]["enum"].clone()
    };
    assert_eq!(offered(&calls[0]), json!(["volume_set", "search_tracks"]));
    assert_eq!(offered(&calls[1]), json!(["volume_set"]));
    assert!(
        !calls[1].body["system"]
            .as_str()
            .unwrap()
            .contains("search_tracks(")
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_spent_user_cap_moves_the_chat_to_the_free_step(pool: PgPool) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let spoken = ["{\"actions\":[],\"say\":\"Привет.\"}"];
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&spoken, 0, 1_000, 100), End::Closes),
    );
    vendor.model(
        "free-model",
        Reply::Stream(vendor::completion(&spoken, 0), End::Closes),
    );
    let nats =
        NatsService::connect(&nats_url(), tokio_util::sync::CancellationToken::new()).await?;
    let bucket = format!("spend_test_{}", Uuid::now_v7().simple());
    let bucket = &bucket[..32];
    let prices = "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0";
    let llm = provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "paid-model",
        prices,
    ) + &provider(
        "free",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "free-model",
        "",
    ) + &ladder(
        &["paid", "free"],
        "\n[ladders.chat.budget]\nuser_day_usd = 0.0028\n",
    ) + &format!("\n[budget]\nbucket = \"{bucket}\"\n")
        + CHAT_SECTION;
    let app = chat_app(&pool, tuned(|_| {}), true, &llm, Some(&nats)).await?;
    let user = fresh_user();
    let session = session_for(&pool, &user).await?;

    assert_eq!(
        ask(&app, session, &turn()).await?.named().last().unwrap().0,
        "done"
    );
    let counters = nats.jetstream().get_key_value(bucket).await?;
    let mut keys = counters.keys().await?;
    let key = keys.next().await.expect("the spend is counted")?;
    assert!(key.starts_with(&format!("chat.user.{user}.day.")), "{key}");
    assert_eq!(counters.get(key).await?.unwrap(), ANSWER_COST_MICRO_USD);

    assert_eq!(
        ask(&app, session, &turn()).await?.named().last().unwrap().0,
        "done"
    );
    assert_eq!(vendor.calls_of("paid-model").len(), 1);
    assert_eq!(vendor.calls_of("free-model").len(), 1);

    let other = session_for(&pool, &fresh_user()).await?;
    assert_eq!(ask(&app, other, &turn()).await?.status, StatusCode::OK);
    assert_eq!(vendor.calls_of("paid-model").len(), 2);
    nats.jetstream().delete_key_value(bucket).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_local_model_answers_over_nats_when_the_http_step_is_down(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model("down-model", Reply::Status(529));
    let lane = lane().await;
    let reply = json!({
        "ok": true,
        "data": {"text": "{\"actions\":[],\"say\":\"Локально.\"}", "model": "local-example", "input_tokens": 40, "output_tokens": 9},
    });
    let worker = lane.worker(Some(reply)).await;
    let local = format!(
        "[providers.local]\nkind = \"worker\"\nsubject = \"{}\"\nmax_output_tokens = 400\ntotal_timeout_ms = 5000\n\n",
        lane.subject
    );
    let llm = provider(
        "down",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "down-model",
        "",
    ) + &local
        + &ladder(&["down", "local"], "")
        + CHAT_SECTION;
    let app = chat_app(&pool, tuned(|_| {}), true, &llm, Some(&lane.nats)).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let heard = ask(&app, session, &turn()).await?;

    assert_eq!(
        heard.named(),
        [("say", &json!({"text": "Локально."})), ("done", &json!({}))]
    );
    let call: Value = serde_json::from_slice(&worker.await?.payload)?;
    assert_eq!(call["max_tokens"], 400);
    assert_eq!(
        call["messages"][2],
        json!({"role": "user", "content": "сделай погромче"})
    );
    assert!(
        call["system"]
            .as_str()
            .unwrap()
            .contains("A cheerful voice named Loli.")
    );
    assert_eq!(call["schema"]["required"], json!(["actions", "say"]));
    lane.close().await;
    Ok(())
}
