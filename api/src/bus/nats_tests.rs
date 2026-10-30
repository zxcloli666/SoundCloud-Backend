use std::time::Duration;

use serde_json::{Value, json};
use tls_common::ClientIdentity;
use tokio_util::sync::CancellationToken;

use super::{NatsRequestError, NatsService};

#[tokio::test]
async fn connect_returns_while_the_server_is_down() {
    let started = std::time::Instant::now();
    NatsService::connect("nats://127.0.0.1:1", CancellationToken::new())
        .await
        .expect("an unreachable server must not fail the connect");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn request_times_out_while_the_server_is_down() {
    let nats = NatsService::connect("nats://127.0.0.1:1", CancellationToken::new())
        .await
        .unwrap();
    let result: Result<Value, _> = nats
        .request("b9.down", &json!({}), Duration::from_millis(300))
        .await;
    assert!(
        matches!(result, Err(NatsRequestError::Timeout(_))),
        "{result:?}"
    );
}

#[tokio::test]
#[ignore = "needs NATS_URL and NATS_TLS_CA, NATS_TLS_CERT, NATS_TLS_KEY of a verify:true server"]
async fn mtls_request_round_trip() {
    tls_common::init_crypto();
    let url = std::env::var("NATS_URL").expect("NATS_URL");
    let identity = ClientIdentity::from_env("NATS").expect("NATS_TLS_* paths");
    let nats = NatsService::connect_with(&url, Some(&identity), CancellationToken::new())
        .await
        .unwrap();

    let responder = nats.clone();
    let mut subscription = nats.subscribe("b9.echo").await.unwrap();
    tokio::spawn(async move {
        use futures::StreamExt;
        while let Some(message) = subscription.next().await {
            if let Some(reply) = message.reply {
                let _ = responder
                    .publish_core(reply.as_str(), message.payload.clone())
                    .await;
            }
        }
    });

    let echoed: Value = nats
        .request("b9.echo", &json!({"n": 7}), Duration::from_secs(3))
        .await
        .expect("round trip over mTLS");
    assert_eq!(echoed, json!({"n": 7}));

    let nobody: Result<Value, _> = nats
        .request("b9.nobody", &json!({}), Duration::from_secs(3))
        .await;
    assert!(
        matches!(nobody, Err(NatsRequestError::NoResponders(_))),
        "{nobody:?}"
    );

    let undecodable: Result<Vec<u8>, _> = nats
        .request("b9.echo", &json!({"n": 1}), Duration::from_secs(3))
        .await;
    assert!(
        matches!(undecodable, Err(NatsRequestError::Decode(..))),
        "{undecodable:?}"
    );
}

#[tokio::test]
#[ignore = "needs NATS_URL of a verify:true server"]
async fn a_client_without_a_certificate_is_refused() {
    tls_common::init_crypto();
    let url = std::env::var("NATS_URL").expect("NATS_URL");
    let nats = NatsService::connect_with(&url, None, CancellationToken::new())
        .await
        .unwrap();
    let result: Result<Value, _> = nats
        .request("b9.echo", &json!({}), Duration::from_secs(2))
        .await;
    assert!(result.is_err(), "{result:?}");
}
