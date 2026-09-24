use std::net::SocketAddr;
use std::time::Duration;

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::source_tree::{read, sources};

const CHUNKS: usize = 6;
const GAP: Duration = Duration::from_millis(120);
const BUDGET: Duration = Duration::from_millis(300);

async fn trickling_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port is free");
    let address = listener.local_addr().expect("the listener has an address");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut request = [0_u8; 1024];
        let _ = socket.read(&mut request).await;
        let header = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: audio/mp4\r\ncontent-length: {CHUNKS}\r\n\r\n"
        );
        if socket.write_all(header.as_bytes()).await.is_err() {
            return;
        }
        for _ in 0..CHUNKS {
            tokio::time::sleep(GAP).await;
            if socket.write_all(b"x").await.is_err() {
                return;
            }
            let _ = socket.flush().await;
        }
    });
    address
}

async fn drain(client: wreq::Client, address: SocketAddr) -> Result<usize, wreq::Error> {
    let response = client.get(format!("http://{address}/track")).send().await?;
    let mut body = response.bytes_stream();
    let mut read = 0;
    while let Some(chunk) = body.next().await {
        read += chunk?.len();
    }
    Ok(read)
}

#[tokio::test]
async fn a_total_timeout_cuts_a_body_that_merely_arrives_slowly() {
    let address = trickling_server().await;
    let client = wreq::Client::builder()
        .timeout(BUDGET)
        .build()
        .expect("the client builds");

    let outcome = drain(client, address).await;

    assert!(
        outcome.is_err(),
        "the upstream was never idle for more than {GAP:?}, yet a total budget of {BUDGET:?} \
         still cut the body after {outcome:?} bytes; this is what truncates a track for a \
         listener on a slow link"
    );
}

#[tokio::test]
async fn a_read_timeout_lets_a_slow_but_living_body_finish() {
    let address = trickling_server().await;
    let client = wreq::Client::builder()
        .read_timeout(BUDGET)
        .build()
        .expect("the client builds");

    let read = drain(client, address).await.expect("the body finishes");

    assert_eq!(
        read, CHUNKS,
        "a body that keeps arriving must be delivered whole, however long it takes in total"
    );
}

fn functions(body: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        let opens = trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn ")
            || trimmed.starts_with("pub(crate) fn ")
            || trimmed.starts_with("pub(crate) async fn ")
            || trimmed.starts_with("pub(super) fn ")
            || trimmed.starts_with("pub(super) async fn ");
        if opens || blocks.is_empty() {
            blocks.push(String::new());
        }
        let current = blocks.last_mut().expect("a block is open");
        current.push_str(line);
        current.push('\n');
    }
    blocks
}

#[test]
fn a_body_handed_to_the_listener_is_never_put_under_a_total_budget() {
    let mut streaming_functions = 0;
    for (path, body) in sources() {
        if path == "src/stream/body_timeout_tests.rs" {
            continue;
        }
        for function in functions(&body) {
            if !function.contains("Body::from_stream") {
                continue;
            }
            streaming_functions += 1;
            assert!(
                !function.contains(".timeout("),
                "{path} hands a body to the listener and puts that same request under a total \
                 budget; the budget keeps running while the listener is still reading, so a slow \
                 link gets a truncated track. Bound the silence with read_timeout instead:\n\
                 {function}"
            );
        }
    }
    assert!(
        streaming_functions > 0,
        "nothing streams a body any more; this guard is reading the wrong tree"
    );
}

#[test]
fn the_client_that_streams_to_the_listener_still_gives_up_on_silence() {
    let main = read("src/main.rs");
    let builder = main
        .split("let storage_passthrough")
        .nth(1)
        .and_then(|rest| rest.split_once("expect("))
        .map(|(builder, _)| builder.to_owned())
        .expect("main.rs builds a passthrough client for storage");

    assert!(
        builder.contains("read_timeout"),
        "the passthrough client has no read budget, so an upstream that goes quiet \
         holds the listener's connection with nothing to end it: {builder}"
    );
    assert!(
        !builder.contains(".timeout("),
        "the passthrough client is back under a total budget: {builder}"
    );
}

#[tokio::test]
async fn a_read_timeout_still_gives_up_on_an_upstream_that_went_quiet() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port is free");
    let address = listener.local_addr().expect("the listener has an address");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut request = [0_u8; 1024];
        let _ = socket.read(&mut request).await;
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: audio/mp4\r\ncontent-length: 4\r\n\r\nx")
            .await;
        let _ = socket.flush().await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let client = wreq::Client::builder()
        .read_timeout(BUDGET)
        .build()
        .expect("the client builds");

    let outcome = tokio::time::timeout(Duration::from_secs(5), drain(client, address)).await;

    assert!(
        matches!(outcome, Ok(Err(_))),
        "an upstream that sent one byte and went silent must be dropped by the read budget, \
         not held until something else notices: {outcome:?}"
    );
}
