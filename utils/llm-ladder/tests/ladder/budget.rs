use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::vendor::{self, COMPLETIONS, MESSAGES, Reply, Seen, Vendor};
use crate::{ANSWER, NoHostSections};
use async_nats::jetstream::{self, kv};
use futures::future::join_all;
use futures::{StreamExt, TryStreamExt};
use llm_ladder::{
    Event, Hosting, Ladder, LadderError, Ladders, Message, Request, Schema, Settings, System,
};

const ANSWER_COST: u64 = 2_800;
const USERS: usize = 3;
const ANSWERS_PER_USER: usize = 12;

struct Host {
    ladders: Ladders,
    seen: Arc<Seen>,
}

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

async fn context() -> jetstream::Context {
    jetstream::new(async_nats::connect(nats_url()).await.expect("nats"))
}

fn config(vendor: &Vendor, bucket: &str, caps: &str) -> String {
    format!(
        "[providers.paid]\nkind = \"anthropic_compatible\"\nendpoint = \"{messages}\"\nmodel = \"paid-model\"\n\
         api_key = \"env:CARGO_PKG_NAME\"\nstream = false\nmax_output_tokens = 600\ntotal_timeout_ms = 4000\n\
         input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\n\n\
         [providers.free]\nkind = \"openai_compatible\"\nendpoint = \"{completions}\"\nmodel = \"free-model\"\n\
         api_key = \"env:CARGO_PKG_NAME\"\nstream = false\nmax_output_tokens = 600\ntotal_timeout_ms = 4000\n\n\
         [ladders.chat]\nsteps = [\"paid\", \"free\"]\nhedge_after_ms = 3000\ndeadline_ms = 8000\n\
         breaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n\n[ladders.chat.budget]\n{caps}\n\n\
         [ladders.strict]\nsteps = [\"paid\"]\nhedge_after_ms = 3000\ndeadline_ms = 8000\n\
         breaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n\n[ladders.strict.budget]\n{caps}\n\n\
         [budget]\nbucket = \"{bucket}\"\n",
        messages = vendor.url(MESSAGES),
        completions = vendor.url(COMPLETIONS),
    )
}

async fn vendor() -> Vendor {
    let vendor = Vendor::open().await;
    vendor.model(
        "paid-model",
        Reply::Whole(0, vendor::whole_message(ANSWER, 1_000, 100)),
    );
    vendor.model(
        "free-model",
        Reply::Whole(0, vendor::whole_completion(ANSWER, Some((1_000, 100)))),
    );
    vendor
}

async fn host(config: &str) -> Host {
    let (settings, _): (Settings, NoHostSections) =
        Settings::parse(config).expect("a valid config");
    let seen = Arc::new(Seen::default());
    let hosting = Hosting {
        jetstream: Some(context().await),
        worker: None,
        observer: seen.clone(),
    };
    Host {
        ladders: Ladders::build(&settings, hosting).expect("the ladders are built"),
        seen,
    }
}

fn request(user: &str) -> Request {
    Request {
        system: System::fixed("Answer with one JSON object."),
        messages: vec![Message::user("hi")],
        schema: Schema::object([("say", Schema::String)]),
        budget_user: Some(user.to_owned()),
        deadline: None,
    }
}

async fn winner(ladder: &Arc<Ladder>, user: &str) -> Result<String, LadderError> {
    let mut events = ladder.stream(request(user)).await?;
    while let Some(event) = events.next().await {
        if let Event::Done(outcome) = event? {
            return Ok(outcome.provider);
        }
    }
    Err(LadderError::ProviderFailed)
}

async fn spent(store: &kv::Store, prefix: &str) -> u64 {
    let keys: Vec<String> = store.keys().await.unwrap().try_collect().await.unwrap();
    let mut total = 0;
    for key in keys.iter().filter(|key| key.starts_with(prefix)) {
        let value = store.get(key.as_str()).await.unwrap().unwrap();
        total += std::str::from_utf8(&value).unwrap().parse::<u64>().unwrap();
    }
    total
}

async fn settles(store: &kv::Store, prefix: &str, expected: u64) -> u64 {
    let limit = Instant::now() + Duration::from_secs(10);
    loop {
        let total = spent(store, prefix).await;
        if total == expected || Instant::now() > limit {
            return total;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a local NATS with JetStream"]
async fn concurrent_priced_answers_are_counted_once_each_for_the_ladder_and_the_user() {
    let vendor = vendor().await;
    let bucket = bucket();
    let host = host(&config(
        &vendor,
        &bucket,
        "month_usd = 100.0\nuser_day_usd = 10.0",
    ))
    .await;
    let ladder = host.ladders.get("chat").unwrap();

    let answers = (0..USERS * ANSWERS_PER_USER).map(|index| {
        let ladder = ladder.clone();
        tokio::spawn(async move { winner(&ladder, &format!("user{}", index % USERS)).await })
    });
    for answer in join_all(answers).await {
        assert_eq!(answer.unwrap().as_deref(), Ok("paid"));
    }

    let jetstream = context().await;
    let store = jetstream.get_key_value(&bucket).await.unwrap();
    let all = (USERS * ANSWERS_PER_USER) as u64 * ANSWER_COST;
    assert_eq!(settles(&store, "chat.month.", all).await, all);
    for user in 0..USERS {
        let own = ANSWERS_PER_USER as u64 * ANSWER_COST;
        let prefix = format!("chat.user.user{user}.day.");
        assert_eq!(settles(&store, &prefix, own).await, own, "{prefix}");
    }
    assert_eq!(
        spent(&store, "chat.day.").await,
        0,
        "a period without a cap is not counted"
    );
    assert_eq!(spent(&store, "chat.user.user0.month.").await, 0);
    assert_eq!(vendor.calls_of("free-model").len(), 0);

    jetstream.delete_key_value(&bucket).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a local NATS with JetStream"]
async fn a_cap_reached_on_one_host_moves_that_user_to_free_steps_on_every_host() {
    let vendor = vendor().await;
    let bucket = bucket();
    let config = config(&vendor, &bucket, "user_day_usd = 0.0028");
    let first_host = host(&config).await;
    let second_host = host(&config).await;

    let first_ladder = first_host.ladders.get("chat").unwrap();
    assert_eq!(winner(&first_ladder, "ann").await.as_deref(), Ok("paid"));
    let jetstream = context().await;
    let store = jetstream.get_key_value(&bucket).await.unwrap();
    assert_eq!(
        settles(&store, "chat.user.ann.day.", ANSWER_COST).await,
        ANSWER_COST
    );

    let second_ladder = second_host.ladders.get("chat").unwrap();
    assert_eq!(winner(&second_ladder, "ann").await.as_deref(), Ok("free"));
    assert_eq!(
        second_host.seen.results(),
        [
            ("paid".to_owned(), "skipped_budget"),
            ("free".to_owned(), "won")
        ]
    );
    assert_eq!(winner(&second_ladder, "bob").await.as_deref(), Ok("paid"));
    assert_eq!(vendor.calls_of("paid-model").len(), 2);
    assert_eq!(
        spent(&store, "chat.user.ann.day.").await,
        ANSWER_COST,
        "a free answer adds nothing"
    );

    let strict = second_host.ladders.get("strict").unwrap();
    assert_eq!(
        winner(&strict, "ann").await.as_deref(),
        Ok("paid"),
        "another ladder has its own counters"
    );
    assert_eq!(
        settles(&store, "strict.user.ann.day.", ANSWER_COST).await,
        ANSWER_COST
    );
    assert_eq!(winner(&strict, "ann").await, Err(LadderError::Unavailable));

    jetstream.delete_key_value(&bucket).await.unwrap();
}

#[tokio::test]
async fn caps_without_a_jetstream_connection_stop_the_start() {
    let vendor = vendor().await;
    let config = config(&vendor, "spend_test_unused", "month_usd = 100.0");
    let (settings, _): (Settings, NoHostSections) = Settings::parse(&config).unwrap();
    let hosting = Hosting {
        jetstream: None,
        worker: None,
        observer: Arc::new(Seen::default()),
    };
    let refused = Ladders::build(&settings, hosting)
        .err()
        .expect("the start is refused");
    assert!(refused.to_string().contains("JetStream"), "{refused}");
}
