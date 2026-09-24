use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::modules::auth::{AuthHealthService, AuthService, TokenKind, TokenProvider};
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::sc::{ScClient, ScReadService};
use sc_transport::{Bytes, RelayTransport};

enum Scripted {
    Answers(Value),
    SaysMissing,
    Silent,
}

struct ScriptedRelay {
    script: Scripted,
    calls: AtomicUsize,
}

impl ScriptedRelay {
    fn new(script: Scripted) -> Arc<Self> {
        Arc::new(Self {
            script,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RelayTransport for ScriptedRelay {
    fn fetch<'a>(
        &'a self,
        _request: &'a call_relay::Request,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<call_relay::Response, call_relay::Error>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err(call_relay::Error::Disabled) })
    }

    fn call_method_rotated<'a>(
        &'a self,
        _method_id: &'a str,
        _script: &'a str,
        _inputs: Bytes,
        _region_rotation: i32,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Bytes, call_relay::Error>> + Send + 'a>,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let body = match &self.script {
            Scripted::Answers(entity) => {
                Some(json!({"ok": true, "track": entity.clone()}).to_string())
            }
            Scripted::SaysMissing => Some(json!({"ok": false}).to_string()),
            Scripted::Silent => None,
        };
        Box::pin(async move {
            match body {
                Some(body) => Ok(Bytes::from(body)),
                None => Err(call_relay::Error::Disabled),
            }
        })
    }
}

fn read_service(pool: &PgPool, relay: Arc<ScriptedRelay>) -> anyhow::Result<Arc<ScReadService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: String::new(),
        proxy_fallback: false,
        api_base: Some("http://127.0.0.1:1".to_owned()),
        home_base: Some("http://127.0.0.1:1".to_owned()),
    })?
    .with_relay(relay);
    let auth = AuthService::new(
        pool.clone(),
        sc.clone(),
        OAuthAppsService::new(pool.clone()),
        AuthHealthService::with_database(redis, pool.clone()),
    );
    let tokens = TokenProvider::new(auth, OAuthAppTokenService::new(pool.clone()));
    Ok(ScReadService::new(sc, tokens, pool.clone()))
}

fn remote_track() -> Value {
    json!({"id":42,"urn":"soundcloud:tracks:42","kind":"track","title":"Relay title","duration":120000})
}

#[sqlx::test(migrations = "./migrations")]
async fn a_relay_answer_ends_the_walk_and_never_reaches_the_token_chain(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = ScriptedRelay::new(Scripted::Answers(remote_track()));
    let read = read_service(&pool, relay.clone())?;

    let track = tokio::time::timeout(
        Duration::from_secs(5),
        read.track_by_id(TokenKind::PublicPool, "42"),
    )
    .await??;

    assert_eq!(track["title"], "Relay title");
    assert_eq!(
        relay.calls(),
        1,
        "the lua tier must be the one that answered"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_relay_that_says_the_entity_is_missing_still_hands_the_walk_on(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = ScriptedRelay::new(Scripted::SaysMissing);
    let read = read_service(&pool, relay.clone())?;

    let failure = tokio::time::timeout(
        Duration::from_secs(5),
        read.track_by_id(TokenKind::PublicPool, "42"),
    )
    .await?
    .expect_err("with no token and no upstream the walk cannot succeed");

    assert!(relay.calls() >= 1, "the lua tier must have been asked");
    assert!(
        !failure.to_string().contains("relay: entity not found"),
        "a relay miss is not the final answer, the walk must continue past it, saw {failure}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_silent_relay_hands_the_walk_on_and_then_stops_being_asked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = ScriptedRelay::new(Scripted::Silent);
    let read = read_service(&pool, relay.clone())?;

    for _ in 0..4 {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            read.track_by_id(TokenKind::PublicPool, "42"),
        )
        .await?;
    }
    let tried = relay.calls();
    assert!(tried > 0, "the first reads must actually try the relay");

    for _ in 0..4 {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            read.track_by_id(TokenKind::PublicPool, "42"),
        )
        .await?;
    }
    assert_eq!(
        relay.calls(),
        tried,
        "an open breaker must send the whole walk straight to the backup tier"
    );
    Ok(())
}
