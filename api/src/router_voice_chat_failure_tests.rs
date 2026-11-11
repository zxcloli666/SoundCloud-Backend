use serde_json::json;
use sqlx::PgPool;

use super::super::session_for;
use super::vendor::{self, COMPLETIONS, End, MESSAGES, Reply, Vendor};
use super::{
    CHAT_SECTION, LOUDER, all_connections_idle, ask, fresh_user, ladder, one_step, open, provider,
    starred, turn,
};
use axum::http::StatusCode;
use futures::StreamExt;

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
        [("error", &json!({"code": "provider_failed"}))],
        "an answer with nothing to do and nothing to say is never streamed"
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

    vendor.model("silent-model", Reply::Stream(Vec::new(), End::Stalls));
    let hurried = provider(
        "silent",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "silent-model",
        "",
    ) + &provider(
        "stalling",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "stalling-model",
        "",
    ) + &ladder(&["silent", "stalling"], "")
        + CHAT_SECTION;
    let hurried = hurried
        .replace("total_timeout_ms = 4000", "total_timeout_ms = 1900")
        .replace("hedge_after_ms = 3000", "hedge_after_ms = 500")
        .replace("deadline_ms = 8000", "deadline_ms = 2000");
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
        ],
        "the hedged step started late and was still within its own timeout at the deadline"
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
async fn an_answer_with_nothing_to_show_falls_through_to_the_next_step(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let blank = ["{\"actions\":[{\"name\":\"open_door\",\"args\":[]}],\"say\":\" \"}"];
    vendor.model(
        "weak-model",
        Reply::Stream(vendor::message(&blank, 0, 1_000, 100), End::Closes),
    );
    vendor.model(
        "strong-model",
        Reply::Stream(vendor::completion(&LOUDER, 0), End::Closes),
    );
    let llm = provider(
        "weak",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "weak-model",
        "",
    ) + &provider(
        "strong",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "strong-model",
        "",
    ) + &ladder(&["weak", "strong"], "")
        + CHAT_SECTION;
    let app = starred(&pool, &llm).await?;
    let session = session_for(&pool, &fresh_user()).await?;

    let heard = ask(&app, session, &turn()).await?;

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
    assert_eq!(vendor.calls_of("weak-model").len(), 1);
    assert_eq!(vendor.calls_of("strong-model").len(), 1);
    Ok(())
}
