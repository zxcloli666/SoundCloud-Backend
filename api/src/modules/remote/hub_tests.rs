use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::remote::{CommandKind, RemoteReply, WaitResponse};
use deadpool_redis::{Config, Pool, Runtime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::hub::{Asked, RemoteHub};
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
    first.poll(&user, "pc", 1, None, Duration::from_millis(10)).await;
    second.poll(&user, "pc", 1, None, Duration::from_millis(10)).await;
    for (hub, text) in [(&first, "раз"), (&second, "два"), (&first, "три")] {
        let _ = hub.ask(&user, "alice", text, Duration::from_millis(30)).await;
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
    use backend_contracts::remote::LinkProvider;
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    hub.poll(&user, "pc", 1, None, Duration::from_millis(10)).await;
    let asking = {
        let hub = hub.clone();
        let user = user.clone();
        tokio::spawn(async move { hub.confirm_link(&user, LinkProvider::Salut).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let delivered = hub
        .poll(&user, "pc", 1, Some(0), Duration::from_millis(50))
        .await;
    let command = &delivered.commands[0];
    assert_eq!(command.kind, CommandKind::LinkConfirm);
    assert_eq!(command.source, "salut");
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
    hub.poll(&user, "pc", 1, None, Duration::from_millis(10)).await;
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
    assert!(started.elapsed() < Duration::from_millis(650), "{:?}", started.elapsed());
}
