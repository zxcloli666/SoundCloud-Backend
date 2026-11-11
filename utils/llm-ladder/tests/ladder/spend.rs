use std::time::{Duration, Instant};

use crate::bench::{PIECES, WAIT, any_text, http_provider, ladder};
use crate::budget::{Host, bucket, context, host, request, settles, winner};
use crate::vendor::{self, COMPLETIONS, Reply, Step, Vendor};
use async_nats::jetstream::kv;
use futures::StreamExt;
use llm_ladder::{Event, LadderError, Request};

const PRICE: &str = "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0";
const CAPS: &str = "month_usd = 100.0\nuser_day_usd = 10.0";

fn config(vendor: &Vendor, bucket: &str) -> String {
    let url = vendor.url(COMPLETIONS);
    let mut config = http_provider("paid", "openai_compatible", &url, "paid-model", PRICE)
        + &http_provider("free", "openai_compatible", &url, "free-model", "");
    for (id, steps) in [("race", &["paid", "free"][..]), ("solo", &["paid"][..])] {
        config += &ladder(id, steps, 300);
        config += &format!("[ladders.{id}.budget]\n{CAPS}\n");
    }
    config + &format!("[budget]\nbucket = \"{bucket}\"\n")
}

async fn paid_report(host: &Host, nth: usize) -> Step {
    let limit = Instant::now() + WAIT;
    loop {
        let paid: Vec<Step> = host
            .seen
            .steps()
            .into_iter()
            .filter(|step| step.provider == "paid")
            .collect();
        if let Some(step) = paid.get(nth - 1) {
            return step.clone();
        }
        assert!(Instant::now() < limit, "the paid step was settled");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn counted(store: &kv::Store, ladder: &str, expected: u64) {
    for prefix in [
        format!("{ladder}.month."),
        format!("{ladder}.user.ann.day."),
    ] {
        assert_eq!(
            settles(store, &prefix, expected).await,
            expected,
            "{prefix}"
        );
    }
}

fn estimated(step: &Step, result: &str) -> u64 {
    assert_eq!((step.result, step.basis), (result, "estimated"));
    assert!(step.usage.input_tokens > 0, "{step:?}");
    assert!(step.cost_micro_usd > 0, "{step:?}");
    step.cost_micro_usd
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a local NATS with JetStream"]
async fn a_paid_step_that_reports_no_usage_still_moves_the_spend_counters() {
    let vendor = Vendor::open().await;
    let bucket = bucket();
    let host = host(&config(&vendor, &bucket)).await;
    let race = host.ladders.get("race").unwrap();
    let solo = host.ladders.get("solo").unwrap();
    vendor.model(
        "free-model",
        Reply::Events(vendor::completion_chunks(&PIECES, Some((900, 20)))),
    );

    let quiet = vendor::completion_chunks(&PIECES, None);
    vendor.model("paid-model", Reply::Gated(quiet.clone(), 1));
    assert_eq!(winner(&race, "ann").await.as_deref(), Ok("free"));
    let lost = estimated(&paid_report(&host, 1).await, "lost");
    let jetstream = context().await;
    let store = jetstream.get_key_value(&bucket).await.unwrap();
    counted(&store, "race", lost).await;

    vendor.model("paid-model", Reply::Gated(quiet.clone(), 2));
    let mut events = solo.stream(request("ann"), any_text()).await.unwrap();
    assert!(matches!(events.next().await, Some(Ok(Event::Delta(_)))));
    drop(events);
    let left = estimated(&paid_report(&host, 2).await, "cancelled");
    counted(&store, "solo", left).await;

    vendor.model("paid-model", Reply::Stalls(Vec::new()));
    let short = Request {
        deadline: Some(Duration::from_millis(1_000)),
        ..request("ann")
    };
    let mut events = solo.stream(short, any_text()).await.unwrap();
    assert_eq!(events.next().await, Some(Err(LadderError::Deadline)));
    drop(events);
    let cut = estimated(&paid_report(&host, 3).await, "failed_timeout");
    counted(&store, "solo", left + cut).await;

    vendor.model("paid-model", Reply::Events(quiet));
    assert_eq!(winner(&solo, "ann").await.as_deref(), Ok("paid"));
    let won = estimated(&paid_report(&host, 4).await, "won");
    assert!(won > left, "the answer it sent is charged too");
    counted(&store, "solo", left + cut + won).await;

    jetstream.delete_key_value(&bucket).await.unwrap();
}
