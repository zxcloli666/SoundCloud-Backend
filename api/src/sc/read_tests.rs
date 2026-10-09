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

const HOME: &str = "http://127.0.0.1:1";

type LuaAnswer = Box<dyn Fn(&Value) -> Option<Value> + Send + Sync>;

pub(crate) struct SearchRelay {
    lua: LuaAnswer,
    proxy_status: u16,
    proxy_body: Value,
    lua_inputs: std::sync::Mutex<Vec<Value>>,
    lua_calls: AtomicUsize,
    fetched: std::sync::Mutex<Vec<String>>,
    stalled: bool,
    redirect: Option<String>,
}

impl SearchRelay {
    fn new(lua: Option<Value>, proxy_status: u16, proxy_body: Value) -> Arc<Self> {
        Self::answering(Box::new(move |_| lua.clone()), proxy_status, proxy_body)
    }

    pub(crate) fn answering(lua: LuaAnswer, proxy_status: u16, proxy_body: Value) -> Arc<Self> {
        Arc::new(Self {
            lua,
            proxy_status,
            proxy_body,
            lua_inputs: std::sync::Mutex::default(),
            lua_calls: AtomicUsize::new(0),
            fetched: std::sync::Mutex::default(),
            stalled: false,
            redirect: None,
        })
    }

    pub(crate) fn redirecting(lua: LuaAnswer, location: &str) -> Arc<Self> {
        Arc::new(Self {
            lua,
            proxy_status: 500,
            proxy_body: Value::Null,
            lua_inputs: std::sync::Mutex::default(),
            lua_calls: AtomicUsize::new(0),
            fetched: std::sync::Mutex::default(),
            stalled: false,
            redirect: Some(location.to_owned()),
        })
    }

    pub(crate) fn stalled() -> Arc<Self> {
        Arc::new(Self {
            lua: Box::new(|_| None),
            proxy_status: 500,
            proxy_body: Value::Null,
            lua_inputs: std::sync::Mutex::default(),
            lua_calls: AtomicUsize::new(0),
            fetched: std::sync::Mutex::default(),
            stalled: true,
            redirect: None,
        })
    }

    pub(crate) fn lua_inputs(&self) -> Vec<Value> {
        self.lua_inputs.lock().unwrap().clone()
    }

    pub(crate) fn fetched_urls(&self) -> Vec<String> {
        self.fetched.lock().unwrap().clone()
    }

    pub(crate) fn lua_calls(&self) -> usize {
        self.lua_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn proxy_searches(&self) -> usize {
        self.fetched
            .lock()
            .unwrap()
            .iter()
            .filter(|url| url.contains("/search/"))
            .count()
    }
}

impl RelayTransport for SearchRelay {
    fn fetch<'a>(
        &'a self,
        request: &'a call_relay::Request,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<call_relay::Response, call_relay::Error>>
                + Send
                + 'a,
        >,
    > {
        self.fetched.lock().unwrap().push(request.url.clone());
        if self.stalled {
            return Box::pin(std::future::pending());
        }
        if let Some(location) = self
            .redirect
            .as_ref()
            .filter(|_| request.url.starts_with("https://on.soundcloud.com/"))
        {
            let headers =
                std::collections::HashMap::from([("Location".to_owned(), location.clone())]);
            return Box::pin(async move {
                Ok(call_relay::Response {
                    status: 302,
                    headers,
                    body: Bytes::new(),
                    source_tier: call_relay::Tier::Direct,
                    client_id: None,
                })
            });
        }
        let (status, body) = if request.url.starts_with(HOME) {
            (
                200,
                r#"[{"hydratable":"apiClient","data":{"id":"cid123"}}]"#.to_owned(),
            )
        } else {
            (self.proxy_status, self.proxy_body.to_string())
        };
        Box::pin(async move {
            Ok(call_relay::Response {
                status,
                headers: Default::default(),
                body: Bytes::from(body),
                source_tier: call_relay::Tier::Direct,
                client_id: None,
            })
        })
    }

    fn call_method_rotated<'a>(
        &'a self,
        _method_id: &'a str,
        _script: &'a str,
        inputs: Bytes,
        _region_rotation: i32,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Bytes, call_relay::Error>> + Send + 'a>,
    > {
        self.lua_calls.fetch_add(1, Ordering::SeqCst);
        if self.stalled {
            return Box::pin(std::future::pending());
        }
        let inputs = serde_json::from_slice::<Value>(&inputs).unwrap_or_default();
        let body = (self.lua)(&inputs).map(|answer| answer.to_string());
        self.lua_inputs.lock().unwrap().push(inputs);
        Box::pin(async move {
            match body {
                Some(body) => Ok(Bytes::from(body)),
                None => Err(call_relay::Error::Disabled),
            }
        })
    }
}

pub(crate) fn search_service(
    pool: &PgPool,
    relay: Arc<SearchRelay>,
) -> anyhow::Result<Arc<ScReadService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: String::new(),
        proxy_fallback: false,
        api_base: Some("http://127.0.0.1:2".to_owned()),
        home_base: Some(HOME.to_owned()),
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

fn sc_hit(id: u64) -> Value {
    json!({"id": id, "kind": "track", "title": format!("hit {id}")})
}

#[sqlx::test(migrations = "./migrations")]
async fn a_lua_search_page_is_normalized_and_keeps_the_cursor_chain(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = SearchRelay::new(
        Some(
            json!({"ok": true, "collection": [sc_hit(7), sc_hit(8)], "next_href": "https://api-v2.soundcloud.com/search/tracks?q=x&offset=20&query_urn=soundcloud%3Asearch%3A1"}),
        ),
        500,
        json!({}),
    );
    let read = search_service(&pool, relay.clone())?;
    let cursor = "https://api-v2.soundcloud.com/search/tracks?q=x&offset=20";

    let page = read
        .search(sc_transport::SearchType::Tracks, "x", Some(cursor))
        .await?;

    assert_eq!(page.items[0]["urn"], "soundcloud:tracks:7");
    assert_eq!(page.items[1]["urn"], "soundcloud:tracks:8");
    assert_eq!(
        page.next_href.as_deref(),
        Some(
            "https://api-v2.soundcloud.com/search/tracks?q=x&offset=20&query_urn=soundcloud%3Asearch%3A1"
        )
    );
    let inputs = relay.lua_inputs();
    assert_eq!(inputs[0]["cursor"], cursor);
    assert_eq!(inputs[0]["type"], "tracks");
    assert_eq!(inputs[0]["limit"], 20);
    assert_eq!(relay.proxy_searches(), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_silent_lua_search_falls_through_to_the_proxy_and_then_stops_being_asked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = SearchRelay::new(
        None,
        200,
        json!({"collection": [sc_hit(9), {"title": "no id"}], "next_href": null}),
    );
    let read = search_service(&pool, relay.clone())?;

    let page = read
        .search(sc_transport::SearchType::Users, "x", None)
        .await?;
    assert_eq!(page.items.len(), 2, "nothing SoundCloud sent is dropped");
    assert_eq!(page.items[0]["urn"], "soundcloud:tracks:9");
    assert_eq!(page.next_href, None);
    assert_eq!(relay.proxy_searches(), 1);

    for _ in 0..3 {
        read.search(sc_transport::SearchType::Users, "x", None)
            .await?;
    }
    let asked = relay.lua_calls();
    assert!(read.search_cooldown().await.is_some());
    read.search(sc_transport::SearchType::Users, "x", None)
        .await?;
    assert_eq!(
        relay.lua_calls(),
        asked,
        "an open search breaker must send the search straight to the proxy"
    );

    let lua_before = relay.lua_calls();
    let _ = read.track_by_id(TokenKind::PublicPool, "42").await;
    assert!(
        relay.lua_calls() > lua_before,
        "search failures must not open the relay_lua breaker the entity reads share"
    );
    assert!(
        relay
            .fetched
            .lock()
            .unwrap()
            .iter()
            .filter(|url| url.contains("/search/"))
            .all(|url| url.starts_with("https://api-v2.soundcloud.com/")),
        "search never reaches the apiv1 token tier"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rejected_query_reports_the_soundcloud_status(pool: PgPool) -> anyhow::Result<()> {
    let relay = SearchRelay::new(None, 422, json!({"error": "bad query"}));
    let read = search_service(&pool, relay)?;

    let failure = read
        .search(sc_transport::SearchType::Tracks, "x", None)
        .await
        .expect_err("SoundCloud refused the query");

    assert!(matches!(
        failure,
        crate::error::AppError::ScApi { status: 422, .. }
    ));
    Ok(())
}

const APIV1: &str = "http://127.0.0.1:2";

async fn every_entity_read(read: &ScReadService) -> Vec<Result<Value, crate::error::AppError>> {
    vec![
        read.resolve(TokenKind::PublicPool, "https://soundcloud.com/discover")
            .await,
        read.track_by_id(TokenKind::PublicPool, "42").await,
        read.user_by_id(TokenKind::PublicPool, "42").await,
        read.playlist_meta(TokenKind::PublicPool, "42").await,
    ]
}

#[sqlx::test(migrations = "./migrations")]
async fn an_apiv2_not_found_is_the_answer_and_skips_apiv1(pool: PgPool) -> anyhow::Result<()> {
    for status in [404, 410] {
        let relay = SearchRelay::new(None, status, json!({"error": "gone"}));
        let read = search_service(&pool, relay.clone())?;

        for result in every_entity_read(&read).await {
            let failure = result.expect_err("SoundCloud said the entity is missing");
            assert!(
                matches!(failure, crate::error::AppError::ScApi { status: s, .. } if s == status),
                "saw {failure}"
            );
            assert_eq!(failure.status().as_u16(), status);
        }
        assert!(
            relay
                .fetched_urls()
                .iter()
                .all(|url| !url.starts_with(APIV1)),
            "an authoritative miss never reaches the apiv1 token tier"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_apiv2_server_error_still_falls_back_to_apiv1(pool: PgPool) -> anyhow::Result<()> {
    let relay = SearchRelay::new(None, 503, json!({"error": "busy"}));
    let read = search_service(&pool, relay.clone())?;

    for result in every_entity_read(&read).await {
        let failure = result.expect_err("no token tier can answer in this test");
        assert!(
            !matches!(failure, crate::error::AppError::ScApi { status: 503, .. }),
            "the apiv1 tier must have the last word, saw {failure}"
        );
    }
    Ok(())
}
