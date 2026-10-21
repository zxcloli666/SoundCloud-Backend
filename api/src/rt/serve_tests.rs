use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::drain;

const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const EMPTY_SETTINGS: [u8; 9] = [0, 0, 0, 4, 0, 0, 0, 0, 0];
const SETTINGS_FRAME: u8 = 4;

fn app() -> Router {
    Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(400)).await;
                "finished"
            }),
        )
        .route("/stuck", get(std::future::pending::<&'static str>))
}

struct Running {
    addr: SocketAddr,
    shutdown: CancellationToken,
    server: JoinHandle<std::io::Result<()>>,
}

async fn started(grace: Duration) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let addr = listener.local_addr().expect("an address");
    let shutdown = CancellationToken::new();
    let server = tokio::spawn(drain(listener, app(), shutdown.clone(), grace));
    Running {
        addr,
        shutdown,
        server,
    }
}

async fn send(addr: SocketAddr, path: &str) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.expect("a connection");
    let request = format!("GET {path} HTTP/1.1\r\nhost: chat-api\r\nconnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("a write");
    stream
}

#[tokio::test]
async fn a_request_in_flight_finishes_after_the_shutdown_signal() {
    let running = started(Duration::from_secs(5)).await;
    let mut stream = send(running.addr, "/slow").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    running.shutdown.cancel();

    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.expect("an answer");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("finished"), "{answer}");

    let stopped = tokio::time::timeout(Duration::from_secs(2), running.server).await;
    assert!(matches!(stopped, Ok(Ok(Ok(())))));
}

#[tokio::test]
async fn work_that_outlives_the_grace_is_cut_at_the_grace() {
    let running = started(Duration::from_millis(300)).await;
    let _stream = send(running.addr, "/stuck").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let cancelled_at = tokio::time::Instant::now();
    running.shutdown.cancel();

    let stopped = tokio::time::timeout(Duration::from_secs(3), running.server).await;
    assert!(matches!(stopped, Ok(Ok(Ok(())))));
    assert!(cancelled_at.elapsed() >= Duration::from_millis(300));
}

#[tokio::test]
async fn haproxy_can_speak_h2_without_tls() {
    let running = started(Duration::from_secs(1)).await;
    let mut stream = TcpStream::connect(running.addr)
        .await
        .expect("a connection");
    stream.write_all(H2_PREFACE).await.expect("a preface");
    stream.write_all(&EMPTY_SETTINGS).await.expect("settings");

    let mut frame = [0u8; 9];
    tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut frame))
        .await
        .expect("an answer in time")
        .expect("a frame header");
    assert_eq!(frame[3], SETTINGS_FRAME);
    running.shutdown.cancel();
}
