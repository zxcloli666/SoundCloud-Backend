use std::sync::Arc;
use std::time::Duration;

use backend_contracts::remote::RemoteReply;
use deadpool_redis::{Config, Pool, Runtime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::hub::{Asked, RemoteHub};
use crate::bus::nats::NatsService;

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
        .wait(&user, "pc", 10, None, Duration::from_secs(1))
        .await;
    let waiting = {
        let host = desktop_host.clone();
        let user = user.clone();
        tokio::spawn(async move {
            host.wait(
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
                .dispatch(&user, "alice", "следующий трек", Duration::from_secs(5))
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
        .wait(&user, "laptop", 5, None, Duration::from_secs(1))
        .await;
    let active = second_host
        .wait(&user, "desktop", 9, None, Duration::from_secs(1))
        .await;
    let laptop = {
        let host = first_host.clone();
        let user = user.clone();
        tokio::spawn(async move {
            host.wait(
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
            host.wait(
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
        .dispatch(&user, "salut", "пауза", Duration::from_millis(300))
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
        .dispatch("no-such-user", "alice", "пауза", Duration::from_secs(3))
        .await;
    assert!(matches!(asked, Asked::Offline));
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_command_sent_between_two_waits_is_picked_up_from_the_buffer() {
    let hub = local_hub();
    let user = Uuid::now_v7().to_string();
    let handshake = hub.wait(&user, "pc", 1, None, Duration::from_secs(1)).await;
    let missed = hub
        .wait(
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
        .dispatch(&user, "alice", "громче", Duration::from_millis(200))
        .await;
    assert!(matches!(asked, Asked::Silent));

    let caught = hub
        .wait(
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
        .wait(
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
        .wait(&user, "pc", 1, None, Duration::from_millis(10))
        .await;
    let asking = {
        let hub = hub.clone();
        let user = user.clone();
        tokio::spawn(async move {
            hub.dispatch(&user, "alice", "пауза", Duration::from_millis(400))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let buffered = hub
        .wait(&user, "pc", 1, Some(0), Duration::from_millis(10))
        .await;
    let id = buffered
        .commands
        .first()
        .map(|command| command.id)
        .expect("a command");
    hub.reply("someone-else", spoken(id, "чужой ответ")).await;
    assert!(matches!(asking.await.expect("asker"), Asked::Silent));
}
