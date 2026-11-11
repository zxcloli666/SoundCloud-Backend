use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_nats::jetstream;
use chrono::Utc;
use futures::future::join_all;

use super::*;
use crate::budget::{Budget, Caps};
use crate::config::CapSettings;

const HOSTS: usize = 4;
const CHARGES_PER_HOST: u64 = 60;

fn nats_url() -> String {
    std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned())
}

fn bucket() -> String {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    format!("spend_test_{micros}")
}

async fn context() -> Context {
    jetstream::new(async_nats::connect(nats_url()).await.expect("nats"))
}

async fn host(bucket: &str) -> Arc<NatsSpend> {
    Arc::new(NatsSpend::new(context().await, bucket))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a local NATS with JetStream"]
async fn concurrent_charges_from_several_hosts_add_up_exactly() {
    let bucket = bucket();
    let mut hosts = Vec::new();
    for _ in 0..HOSTS {
        hosts.push(host(&bucket).await);
    }
    assert_eq!(
        hosts[0]
            .spent("chat.month.2026-03".to_owned())
            .await
            .unwrap(),
        0
    );

    let charges = hosts.iter().flat_map(|host| {
        (1..=CHARGES_PER_HOST).map(move |amount| {
            let host = host.clone();
            tokio::spawn(async move {
                let shared = host.add("chat.month.2026-03".to_owned(), amount).await;
                let own = host
                    .add(format!("chat.user.{amount}.day.2026-03-09"), amount)
                    .await;
                shared.and(own)
            })
        })
    });
    for written in join_all(charges).await {
        written
            .expect("the task finishes")
            .expect("the charge is written");
    }

    let per_host: u64 = (1..=CHARGES_PER_HOST).sum();
    for host in &hosts {
        assert_eq!(
            host.spent("chat.month.2026-03".to_owned()).await.unwrap(),
            per_host * HOSTS as u64
        );
        assert_eq!(
            host.spent("chat.user.7.day.2026-03-09".to_owned())
                .await
                .unwrap(),
            7 * HOSTS as u64
        );
    }
    context().await.delete_key_value(&bucket).await.unwrap();
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn the_bucket_keeps_one_small_value_per_counter_and_forgets_old_ones() {
    let bucket = bucket();
    let spend = host(&bucket).await;
    spend
        .add("chat.day.2026-03-09".to_owned(), 5)
        .await
        .unwrap();
    spend
        .add("chat.day.2026-03-09".to_owned(), 6)
        .await
        .unwrap();

    let jetstream = context().await;
    let mut stream = jetstream.get_stream(format!("KV_{bucket}")).await.unwrap();
    let info = stream.info().await.unwrap();
    assert_eq!(info.config.max_messages_per_subject, 1);
    assert_eq!(info.config.max_age, COUNTER_AGE);
    assert_eq!(info.config.max_bytes, BUCKET_BYTES);
    assert_eq!(info.config.max_message_size, MAX_COUNTER_MESSAGE);
    assert_eq!(info.state.messages, 1);
    let stored = jetstream.get_key_value(&bucket).await.unwrap();
    assert_eq!(
        stored.get("chat.day.2026-03-09").await.unwrap().unwrap(),
        "11"
    );

    jetstream.delete_key_value(&bucket).await.unwrap();
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_counter_that_is_not_a_number_is_reported_and_left_alone() {
    let bucket = bucket();
    let spend = host(&bucket).await;
    spend
        .add("chat.day.2026-03-09".to_owned(), 5)
        .await
        .unwrap();
    let jetstream = context().await;
    let stored = jetstream.get_key_value(&bucket).await.unwrap();
    stored
        .put("chat.day.2026-03-09", "five".into())
        .await
        .unwrap();

    assert!(matches!(
        spend.spent("chat.day.2026-03-09".to_owned()).await,
        Err(SpendError::Corrupt)
    ));
    assert!(matches!(
        spend.add("chat.day.2026-03-09".to_owned(), 1).await,
        Err(SpendError::Corrupt)
    ));
    assert_eq!(
        stored.get("chat.day.2026-03-09").await.unwrap().unwrap(),
        "five"
    );

    jetstream.delete_key_value(&bucket).await.unwrap();
}

#[tokio::test]
async fn an_unreachable_server_spends_the_budget_within_the_read_budget() {
    let client = async_nats::ConnectOptions::new()
        .retry_on_initial_connect()
        .connect("nats://127.0.0.1:1")
        .await
        .unwrap();
    let spend = Arc::new(NatsSpend::new(
        jetstream::new(client),
        "spend_test_unreachable",
    ));
    let caps = Caps::from(CapSettings {
        month_usd: Some(1.0),
        ..CapSettings::default()
    });
    let budget = Budget::new(spend, "chat", caps);
    let started = Instant::now();

    assert!(budget.is_spent(None, Utc::now()).await);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}
