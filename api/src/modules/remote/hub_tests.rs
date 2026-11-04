use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::remote::{
    CommandKind, LinkProvider, RemoteCommand, RemoteReply, WaitResponse,
};
use deadpool_redis::{Config, Pool, Runtime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::buffer::Buffer;
use super::hub::{Asked, RemoteHub, now_micros, now_ms};
use crate::bus::nats::NatsService;

impl RemoteHub {
    async fn poll(
        &self,
        user: &str,
        device: &str,
        active: i64,
        since: Option<i64>,
        hold: Duration,
    ) -> WaitResponse {
        self.wait(user, device, active, since, hold)
            .await
            .expect("a wait slot")
    }

    async fn ask(&self, user: &str, source: &str, text: &str, wait: Duration) -> Asked {
        self.dispatch(
            user,
            CommandKind::Utterance,
            source,
            text,
            Instant::now() + wait,
        )
        .await
    }
}

fn redis() -> Pool {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    Config::from_url(url)
        .create_pool(Some(Runtime::Tokio1))
        .expect("a redis pool")
}

async fn hub_on_nats() -> Arc<RemoteHub> {
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    let nats = NatsService::connect(&url, CancellationToken::new())
        .await
        .expect("nats");
    let hub = RemoteHub::new(
        Some(nats),
        redis(),
        &format!("remote-test-{}", Uuid::now_v7()),
    );
    hub.start().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    hub
}

fn local_hub() -> Arc<RemoteHub> {
    RemoteHub::new(None, redis(), &format!("remote-test-{}", Uuid::now_v7()))
}

async fn until_known(hub: &RemoteHub, user: &str) {
    for _ in 0..100 {
        if hub.active_device(user).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("presence never reached the host");
}

fn spoken(id: Uuid, text: &str) -> RemoteReply {
    RemoteReply {
        id,
        ok: true,
        speech: text.to_owned(),
        display: text.to_owned(),
        state: None,
    }
}

#[tokio::test]
#[ignore = "requires a local Redis and NATS"]
async fn a_command_created_on_one_host_reaches_a_waiter_on_another_and_the_reply_returns() {
    let creator = hub_on_nats().await;
    let desktop_host = hub_on_nats().await;
    let user = Uuid::now_v7().to_string();

    let handshake = desktop_host
        .poll(&user, "pc", 10, None, Duration::from_secs(1))
        .await;
    let waiting = {
        let host = desktop_host.clone();
        let user = user.clone();
        tokio::spawn(async move {
            host.poll(
                &user,
                "pc",
                10,
                Some(handshake.cursor),
                Duration::from_secs(10),
            )
            .await
        })
    };
    until_known(&creator, &user).await;

    let asking = {
        let creator = creator.clone();
        let user = user.clone();
        tokio::spawn(async move {
            creator
                .ask(&user, "alice", "следующий трек", Duration::from_secs(5))
                .await
        })
    };

    let delivered = waiting.await.expect("the waiter finishes");
    assert_eq!(delivered.commands.len(), 1);
    let command = &delivered.commands[0];
    assert_eq!(command.text, "следующий трек");
    assert_eq!(command.source, "alice");
    assert_eq!(delivered.active_device.as_deref(), Some("pc"));

    desktop_host
        .reply(&user, spoken(command.id, "Следующий трек"))
        .await;
    match asking.await.expect("the asker finishes") {
        Asked::Reply(reply) => assert_eq!(reply.speech, "Следующий трек"),
        _ => panic!("the reply did not come back to the creating host"),
    }
}

#[tokio::test]
#[ignore = "requires a local Redis and NATS"]
async fn only_the_most_recently_active_desktop_gets_the_command() {
    let first_host = hub_on_nats().await;
    let second_host = hub_on_nats().await;
    let creator = hub_on_nats().await;
    let user = Uuid::now_v7().to_string();

    let quiet = first_host
        .poll(&user, "laptop", 5, None, Duration::from_secs(1))
        .await;
    let active = second_host
        .poll(&user, "desktop", 9, None, Duration::from_secs(1))
        .await;
    let laptop = {
        let host = first_host.clone();
        let user = user.clone();
        tokio::spawn(async move {
            host.poll(
                &user,
                "laptop",
                5,
                Some(quiet.cursor),
                Duration::from_secs(2),
            )
            .await
        })
    };
    let desktop = {
        let host = second_host.clone();
        let user = user.clone();
        tokio::spawn(async move {
            host.poll(
                &user,
                "desktop",
                9,
                Some(active.cursor),
                Duration::from_secs(2),
            )
            .await
        })
    };
    until_known(&creator, &user).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let _ = creator
        .ask(&user, "salut", "пауза", Duration::from_millis(300))
        .await;
    assert!(laptop.await.expect("laptop").commands.is_empty());
    assert_eq!(desktop.await.expect("desktop").commands.len(), 1);
}

#[tokio::test]
#[ignore = "requires a local Redis and NATS"]
async fn a_user_nobody_is_polling_for_is_offline_at_once() {
    let hub = hub_on_nats().await;
    let started = std::time::Instant::now();
    let asked = hub
        .ask("no-such-user", "alice", "пауза", Duration::from_secs(3))
        .await;
    assert!(matches!(asked, Asked::Offline));
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_command_sent_between_two_waits_is_picked_up_from_the_buffer() {
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    let handshake = hub.poll(&user, "pc", 1, None, Duration::from_secs(1)).await;
    let missed = hub
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert!(missed.commands.is_empty());
    assert_eq!(missed.cursor, handshake.cursor);

    let asked = hub
        .ask(&user, "alice", "громче", Duration::from_millis(200))
        .await;
    assert!(matches!(asked, Asked::Silent));

    let caught = hub
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert_eq!(caught.commands.len(), 1);
    assert_eq!(caught.commands[0].text, "громче");
    let again = hub
        .poll(
            &user,
            "pc",
            1,
            Some(caught.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert!(again.commands.is_empty());
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_reply_from_another_user_is_ignored() {
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    let _ = hub
        .poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    let asking = {
        let hub = hub.clone();
        let user = user.clone();
        tokio::spawn(async move {
            hub.ask(&user, "alice", "пауза", Duration::from_millis(400))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let buffered = hub
        .poll(&user, "pc", 1, Some(0), Duration::from_millis(10))
        .await;
    let id = buffered
        .commands
        .first()
        .map(|command| command.id)
        .expect("a command");
    hub.reply("someone-else", spoken(id, "чужой ответ")).await;
    assert!(matches!(asking.await.expect("asker"), Asked::Silent));
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn two_hosts_on_one_redis_number_a_users_commands_in_order() {
    let prefix = format!("remote-test-{}", Uuid::now_v7());
    let first = RemoteHub::new(None, redis(), &prefix);
    let second = RemoteHub::new(None, redis(), &prefix);
    let user = Uuid::now_v7().to_string();
    first
        .poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    second
        .poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    for (hub, text) in [(&first, "раз"), (&second, "два"), (&first, "три")] {
        let _ = hub
            .ask(&user, "alice", text, Duration::from_millis(30))
            .await;
    }
    let caught = first
        .poll(&user, "pc", 1, Some(0), Duration::from_millis(50))
        .await;
    let texts: Vec<&str> = caught.commands.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["раз", "два", "три"]);
    let seqs: Vec<i64> = caught.commands.iter().map(|c| c.seq).collect();
    assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_link_confirmation_is_a_command_of_its_own_kind() {
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    hub.poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    let asking = {
        let hub = hub.clone();
        let user = user.clone();
        tokio::spawn(async move { hub.confirm_link(&user, LinkProvider::Salut, "47").await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let delivered = hub
        .poll(&user, "pc", 1, Some(0), Duration::from_millis(50))
        .await;
    let command = &delivered.commands[0];
    assert_eq!(command.kind, CommandKind::LinkConfirm);
    assert_eq!(command.source, "salut");
    assert_eq!(command.hint.as_deref(), Some("47"));
    assert!(command.expires_at - command.created_at > 55_000);
    hub.reply(&user, spoken(command.id, "")).await;
    match asking.await.expect("asker") {
        Asked::Reply(reply) => assert!(reply.ok),
        _ => panic!("no answer"),
    }
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn the_deadline_counts_from_the_request_start_not_from_the_dispatch() {
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    hub.poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    let started = Instant::now();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = hub
        .dispatch(
            &user,
            CommandKind::Utterance,
            "alice",
            "x",
            started + Duration::from_millis(500),
        )
        .await;
    assert!(
        started.elapsed() < Duration::from_millis(650),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_dropped_ask_leaves_no_pending_entry() {
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    hub.poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    let asking = {
        let hub = hub.clone();
        let user = user.clone();
        tokio::spawn(async move {
            hub.ask(&user, "alice", "пауза", Duration::from_secs(5))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(hub.pending_count(), 1);
    asking.abort();
    let _ = asking.await;
    assert_eq!(hub.pending_count(), 0, "an aborted request cleans up");
}

#[tokio::test]
async fn presence_goes_out_only_for_a_new_device_a_new_activity_or_a_stale_entry() {
    let hub = local_hub();
    let start = Instant::now();
    assert!(hub.presence_due("1", "pc", 5, start));
    assert!(!hub.presence_due("1", "pc", 5, start + Duration::from_secs(1)));
    assert!(hub.presence_due("1", "laptop", 5, start + Duration::from_secs(1)));
    assert!(hub.presence_due("1", "pc", 6, start + Duration::from_secs(2)));
    assert!(!hub.presence_due("1", "pc", 6, start + Duration::from_secs(3)));
    let stale = start + Duration::from_secs(2) + super::hub::PRESENCE_REFRESH;
    assert!(hub.presence_due("1", "pc", 6, stale));
}

#[tokio::test]
#[ignore = "requires a local Redis and NATS"]
async fn a_polling_loop_publishes_presence_once() {
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    let watcher = async_nats::connect(&url).await.expect("nats");
    let mut presence = watcher
        .subscribe(backend_contracts::remote::PRESENCE_SUBJECT)
        .await
        .expect("a presence subscription");
    watcher.flush().await.expect("flush");
    let hub = hub_on_nats().await;
    let user = Uuid::now_v7().to_string();
    for _ in 0..50 {
        hub.poll(&user, "pc", 7, None, Duration::from_millis(1))
            .await;
    }
    let mut seen = 0;
    while let Ok(Some(message)) = tokio::time::timeout(
        Duration::from_millis(500),
        futures::StreamExt::next(&mut presence),
    )
    .await
    {
        let parsed: backend_contracts::remote::Presence =
            serde_json::from_slice(&message.payload).expect("presence json");
        if parsed.user == user {
            seen += 1;
        }
    }
    assert_eq!(seen, 1, "fifty polls with the same activity publish once");
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_stuck_redis_does_not_hold_back_live_delivery() {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let mut config = Config::from_url(url);
    config.pool = Some(deadpool_redis::PoolConfig {
        max_size: 1,
        timeouts: deadpool_redis::Timeouts {
            wait: Some(Duration::from_secs(3)),
            create: Some(Duration::from_secs(1)),
            recycle: Some(Duration::from_secs(1)),
        },
        ..Default::default()
    });
    let pool = config.create_pool(Some(Runtime::Tokio1)).expect("a pool");
    let hub = RemoteHub::new(
        None,
        pool.clone(),
        &format!("remote-test-{}", Uuid::now_v7()),
    );
    let user = Uuid::now_v7().to_string();
    let handshake = hub
        .poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    let waiting = {
        let hub = hub.clone();
        let user = user.clone();
        tokio::spawn(async move {
            hub.poll(
                &user,
                "pc",
                1,
                Some(handshake.cursor),
                Duration::from_secs(5),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let held = pool.get().await.expect("the only connection");
    let now = now_ms();
    let started = Instant::now();
    hub.receive(backend_contracts::remote::RemoteCommand {
        id: Uuid::now_v7(),
        kind: CommandKind::Utterance,
        seq: now_micros(),
        user: user.clone(),
        source: "alice".to_owned(),
        text: "пауза".to_owned(),
        hint: None,
        created_at: now,
        expires_at: now + 5_000,
    })
    .await;
    let delivered = waiting.await.expect("the waiter");
    assert_eq!(delivered.commands.len(), 1);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    drop(held);
}

const SKEW_MICROS: i64 = 5_000_000;

fn hub_and_buffer() -> (Arc<RemoteHub>, Buffer) {
    let prefix = format!("remote-test-{}", Uuid::now_v7());
    (
        RemoteHub::new(None, redis(), &prefix),
        Buffer::new(redis(), prefix),
    )
}

fn command_numbered(user: &str, seq: i64, text: &str) -> RemoteCommand {
    let now = now_ms();
    RemoteCommand {
        id: Uuid::now_v7(),
        kind: CommandKind::Utterance,
        seq,
        user: user.to_owned(),
        source: "alice".to_owned(),
        text: text.to_owned(),
        hint: None,
        created_at: now,
        expires_at: now + 5_000,
    }
}

fn texts(response: &WaitResponse) -> Vec<&str> {
    response
        .commands
        .iter()
        .map(|command| command.text.as_str())
        .collect()
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_host_with_a_slow_clock_still_reaches_a_desktop_that_just_started() {
    let (hub, buffer) = hub_and_buffer();
    let user = Uuid::now_v7().to_string();
    let handshake = hub
        .poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;

    let seq = buffer
        .next_seq(&user, now_micros() - SKEW_MICROS)
        .await
        .expect("a sequence");
    buffer
        .push(&command_numbered(&user, seq, "пауза"))
        .await
        .expect("buffered");

    let caught = hub
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert_eq!(texts(&caught), ["пауза"]);
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_command_numbered_by_a_fast_clock_before_the_start_is_not_delivered() {
    let (hub, buffer) = hub_and_buffer();
    let user = Uuid::now_v7().to_string();
    let seq = buffer
        .next_seq(&user, now_micros() + SKEW_MICROS)
        .await
        .expect("a sequence");
    buffer
        .push(&command_numbered(&user, seq, "старая команда"))
        .await
        .expect("buffered");

    let handshake = hub
        .poll(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    assert_eq!(handshake.cursor, seq);
    let quiet = hub
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert!(quiet.commands.is_empty());

    let _ = hub
        .ask(&user, "alice", "новая команда", Duration::from_millis(30))
        .await;
    let caught = hub
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert_eq!(texts(&caught), ["новая команда"]);
    assert!(caught.cursor > seq);
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn an_answered_link_confirm_never_reaches_a_desktop_that_takes_over_on_another_host() {
    let prefix = format!("remote-test-{}", Uuid::now_v7());
    let first = RemoteHub::new(None, redis(), &prefix);
    let second = RemoteHub::new(None, redis(), &prefix);
    let user = Uuid::now_v7().to_string();
    let laptop = second
        .poll(&user, "laptop", 1, None, Duration::from_millis(10))
        .await;
    let desktop = first
        .poll(&user, "desktop", 5, None, Duration::from_millis(10))
        .await;

    let asking = {
        let first = first.clone();
        let user = user.clone();
        tokio::spawn(async move { first.confirm_link(&user, LinkProvider::Alice, "47").await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let shown = first
        .poll(
            &user,
            "desktop",
            5,
            Some(desktop.cursor),
            Duration::from_secs(1),
        )
        .await;
    let command = &shown.commands[0];
    assert_eq!(command.kind, CommandKind::LinkConfirm);
    first.reply(&user, spoken(command.id, "")).await;
    assert!(matches!(asking.await.expect("asker"), Asked::Reply(_)));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let taken_over = second
        .poll(
            &user,
            "laptop",
            9,
            Some(laptop.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert!(taken_over.commands.is_empty(), "{:?}", texts(&taken_over));
    let same_host = first
        .poll(
            &user,
            "desktop",
            9,
            Some(desktop.cursor),
            Duration::from_millis(50),
        )
        .await;
    assert!(same_host.commands.is_empty());
}
