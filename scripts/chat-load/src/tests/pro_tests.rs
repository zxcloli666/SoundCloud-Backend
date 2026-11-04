use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::routing::post;

use super::exchange;
use crate::fake_pro;
use crate::transport::Failure;

async fn served(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("an address");
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    format!("http://{address}")
}

fn origin() -> Router {
    Router::new().route(
        "/chat/x",
        post(|body: Bytes| async move {
            let mut answer = body.to_vec();
            answer.reverse();
            (StatusCode::OK, [("retry-after", "2")], answer)
        }),
    )
}

#[tokio::test]
async fn a_call_through_relay_pro_comes_back_in_pieces() {
    let origin = served(origin()).await;
    let node = served(fake_pro::router().expect("a fake pro")).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("a client");
    let envelope: Vec<u8> = (0..100u8).collect();
    let outer = exchange(
        &client,
        &node,
        &format!("{origin}/chat/x"),
        envelope.clone(),
        Duration::from_secs(10),
        7,
    )
    .await
    .expect("an answer");
    let mut reversed = envelope;
    reversed.reverse();
    assert_eq!(outer.status, 200);
    assert_eq!(outer.body.as_ref(), reversed.as_slice());
    assert_eq!(outer.retry_after, Some(Duration::from_secs(2)));
}

#[tokio::test]
async fn a_dead_pro_node_is_a_transport_failure() {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("a client");
    let failed = exchange(
        &client,
        "http://127.0.0.1:1",
        "http://127.0.0.1:1/chat/x",
        vec![1, 2, 3],
        Duration::from_secs(2),
        8,
    )
    .await;
    assert_eq!(failed.err(), Some(Failure::Connect));
}
