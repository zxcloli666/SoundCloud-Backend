use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use futures::StreamExt;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::super::{cap, session_for, tuned};
use super::vendor::{self, End, MESSAGES, Reply, Vendor};
use super::{
    ANSWER_COST_MICRO_USD, CHAT_SECTION, LOUDER, ask, chat_app, fresh_user, ladder, one_step, open,
    provider, starred, turn,
};
use crate::bus::nats::NatsService;
use crate::modules::voice_chat::testing::{Heard, hear, nats_url};
use crate::router::MAX_BODY_BYTES;

async fn post(
    app: &axum::Router,
    path: &str,
    session: Uuid,
    body: String,
) -> anyhow::Result<Heard> {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-session-id", session.to_string())
        .body(Body::from(body))?;
    hear(app.clone().oneshot(request).await?).await
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
async fn the_star_check_and_the_limit_come_before_the_body_is_read(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let llm = one_step(&vendor, "paid-model");
    let session = session_for(&pool, &fresh_user()).await?;
    let unstarred = chat_app(&pool, tuned(|_| {}), false, &llm, None).await?;

    for body in ["{not json".to_owned(), "x".repeat(2 * MAX_BODY_BYTES)] {
        let heard = post(&unstarred, "/voice/chat", session, body).await?;
        assert_eq!(heard.status, StatusCode::FORBIDDEN);
        assert_eq!(heard.body["code"], "star_required");
    }

    let limited = tuned(|config| config.admission.voice_chat = cap(1));
    let app = chat_app(&pool, limited, true, &llm, None).await?;
    let other = session_for(&pool, &fresh_user()).await?;
    let first = post(&app, "/voice/chat", other, "{not json".to_owned()).await?;
    assert_eq!(first.status, StatusCode::BAD_REQUEST);
    assert_eq!(first.body["message"], "the body is not JSON");
    let second = post(&app, "/voice/chat", other, "{not json".to_owned()).await?;
    assert_eq!(
        second.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a body that cannot be read still counts against the limit"
    );
    assert!(vendor.calls().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_urn_and_a_bare_id_are_one_user_for_the_limit_and_the_budget(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&LOUDER, 0, 1_000, 100), End::Closes),
    );
    let nats =
        NatsService::connect(&nats_url(), tokio_util::sync::CancellationToken::new()).await?;
    let bucket = format!("spend_test_{}", Uuid::now_v7().simple());
    let bucket = &bucket[..32];
    let llm = provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "paid-model",
        "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0",
    ) + &ladder(&["paid"], "\n[ladders.chat.budget]\nuser_day_usd = 1.0\n")
        + &format!("\n[budget]\nbucket = \"{bucket}\"\n")
        + CHAT_SECTION;
    let limited = tuned(|config| config.admission.voice_chat = cap(2));
    let app = chat_app(&pool, limited, true, &llm, Some(&nats)).await?;
    let user = fresh_user();
    let by_urn = session_for(&pool, &format!("soundcloud:users:{user}")).await?;
    let by_id = session_for(&pool, &user).await?;

    assert_eq!(ask(&app, by_urn, &turn()).await?.status, StatusCode::OK);
    assert_eq!(ask(&app, by_id, &turn()).await?.status, StatusCode::OK);
    let third = ask(&app, by_urn, &turn()).await?;
    assert_eq!(third.status, StatusCode::TOO_MANY_REQUESTS);

    let counters = nats.jetstream().get_key_value(bucket).await?;
    let keys: Vec<String> = counters
        .keys()
        .await?
        .filter_map(|key| async { key.ok() })
        .collect()
        .await;
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(
        keys[0].starts_with(&format!("chat.user.{user}.day.")),
        "{keys:?}"
    );
    let spent: u64 = String::from_utf8(counters.get(&keys[0]).await?.unwrap().to_vec())?.parse()?;
    assert_eq!(spent, 2 * ANSWER_COST_MICRO_USD.parse::<u64>()?);
    nats.jetstream().delete_key_value(bucket).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn only_the_chat_path_passes_the_premium_gate_of_a_reserve_host(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let llm = one_step(&vendor, "paid-model");
    let session = session_for(&pool, &fresh_user()).await?;
    let plain = chat_app(&pool, tuned(|_| {}), false, &llm, None).await?;
    let reserve = chat_app(
        &pool,
        tuned(|config| config.premium_reserve = true),
        false,
        &llm,
        None,
    )
    .await?;
    let body = turn().to_string();

    let chat = post(&reserve, "/voice/chat", session, body.clone()).await?;
    assert_eq!(chat.body["code"], "star_required", "the handler answered");
    let unrouted = post(&plain, "/voice/other", session, body.clone()).await?;
    assert_eq!(unrouted.status, StatusCode::NOT_FOUND);
    let gated = post(&reserve, "/voice/other", session, body).await?;
    assert_eq!(
        gated.status,
        StatusCode::FORBIDDEN,
        "the gate answered first"
    );
    assert_ne!(gated.body["code"], "star_required");
    assert!(vendor.calls().is_empty());
    Ok(())
}
