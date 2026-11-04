use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use uuid::Uuid;

use super::hub::{Asked, Refused, RemoteHub, Replied};
use super::hub_tests::{connect, host, nats_url, prefix, spoken, user};

type Slowdown = Arc<Mutex<Option<(String, Duration)>>>;

struct Relay {
    url: String,
    held: watch::Sender<bool>,
    slow: Slowdown,
}

impl Relay {
    async fn start() -> Self {
        let upstream = url::Url::parse(&nats_url()).expect("a NATS url");
        let upstream = format!(
            "{}:{}",
            upstream.host_str().unwrap_or("127.0.0.1"),
            upstream.port().unwrap_or(4222)
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a relay port");
        let url = format!("nats://{}", listener.local_addr().expect("a relay address"));
        let (held, released) = watch::channel(false);
        let slow = Slowdown::default();
        tokio::spawn(accept(listener, upstream, released, slow.clone()));
        Self { url, held, slow }
    }

    fn hold(&self) {
        self.held.send_replace(true);
    }

    fn slow_down(&self, needle: &str, by: Duration) {
        *self.slow.lock().unwrap_or_else(|e| e.into_inner()) = Some((needle.to_owned(), by));
    }
}

async fn accept(
    listener: TcpListener,
    upstream: String,
    released: watch::Receiver<bool>,
    slow: Slowdown,
) {
    while let Ok((client, _)) = listener.accept().await {
        let Ok(server) = TcpStream::connect(&upstream).await else {
            continue;
        };
        let (client_read, client_write) = client.into_split();
        let (server_read, server_write) = server.into_split();
        tokio::spawn(outbound(client_read, server_write, slow.clone()));
        tokio::spawn(inbound(server_read, client_write, released.clone()));
    }
}

async fn outbound(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, slow: Slowdown) {
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = match from.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        let delay = slow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|(needle, _)| contains(&buffer[..read], needle.as_bytes()))
            .map(|(_, by)| *by);
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        if to.write_all(&buffer[..read]).await.is_err() {
            return;
        }
    }
}

async fn inbound(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, mut held: watch::Receiver<bool>) {
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = match from.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        let released = held.wait_for(|held| !*held).await.is_ok();
        if !released || to.write_all(&buffer[..read]).await.is_err() {
            return;
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_server_that_stops_answering_after_the_open_is_given_up_on_and_stays_fast() {
    let relay = Relay::start().await;
    let hub = RemoteHub::new(Some(connect(&relay.url).await), &prefix());
    assert!(hub.prepare().await, "the store opens through the relay");
    let user = user();
    relay.hold();

    let started = Instant::now();
    assert!(matches!(
        hub.wait(&user, "pc", 1, None, Duration::ZERO).await,
        Err(Refused::Unavailable)
    ));
    assert_eq!(
        hub.reply(&user, spoken(Uuid::now_v7(), "")).await,
        Replied::Busy
    );
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "each request gives up after 2 s, not after the 10 s client default: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn an_open_its_caller_gave_up_on_still_counts_as_failed() {
    let relay = Relay::start().await;
    let nats = connect(&relay.url).await;
    assert!(nats.wait_connected(Duration::from_secs(2)).await);
    let hub = RemoteHub::new(Some(nats), &prefix());
    relay.hold();

    let gave_up = tokio::time::timeout(
        Duration::from_millis(200),
        hub.wait("1", "pc", 1, None, Duration::ZERO),
    )
    .await;
    assert!(gave_up.is_err(), "the open is still waiting for the server");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !hub.open_failed() {
        assert!(
            Instant::now() < deadline,
            "the dropped open was never recorded"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let started = Instant::now();
    assert!(matches!(
        hub.wait("1", "pc", 1, None, Duration::ZERO).await,
        Err(Refused::Unavailable)
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_command_with_a_late_publish_ack_still_gets_its_answer() {
    let prefix = prefix();
    let relay = Relay::start().await;
    let asking = RemoteHub::new(Some(connect(&relay.url).await), &prefix);
    assert!(asking.prepare().await, "the store opens through the relay");
    let desktop = host(&prefix).await;
    let user = user();
    let handshake = desktop.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let answering = {
        let desktop = desktop.clone();
        let user = user.clone();
        tokio::spawn(async move {
            let shown = desktop
                .poll(
                    &user,
                    "pc",
                    1,
                    Some(handshake.cursor),
                    Duration::from_secs(6),
                )
                .await;
            let id = shown.commands.first().expect("the slow command arrives").id;
            desktop.answers(&user, spoken(id, "дошло")).await
        })
    };

    relay.slow_down(
        &format!("{prefix}.cmd.{user}"),
        Duration::from_millis(1_500),
    );
    let asked = asking
        .ask(&user, "alice", "медленно", Duration::from_secs(5))
        .await;
    assert!(answering.await.expect("the desktop"), "the answer is taken");
    match asked {
        Asked::Reply(reply) => assert_eq!(reply.speech, "дошло"),
        _ => panic!("the asking host stopped listening when the publish ack was late"),
    }
}
