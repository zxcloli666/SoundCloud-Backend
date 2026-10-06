use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::meta::{LivePage, LiveState};
use super::query::{LiveClass, LiveKind};
use super::service::{LiveRequest, LiveSearch};
use super::serving;
use super::store::{LiveStore, Window};
use crate::cache::{CacheService, ListPageResult};
use crate::common::admission::PublicAdmission;
use crate::common::pagination::PaginationQuery;
use crate::config::{AdmissionCfg, AdmissionLimitCfg, LiveMode, LiveSearchCfg};
use crate::error::AppResult;
use crate::modules::auth::{AuthHealthService, AuthService, TokenProvider};
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::sc::{ScClient, ScReadService};
use sc_transport::{Bytes, RelayTransport};

type RelayFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, call_relay::Error>> + Send + 'a>>;

#[derive(Clone)]
pub(super) enum Script {
    Answers(Vec<Value>),
    Slow(Duration),
    Silent,
}

pub(super) struct SearchRelay {
    pub(super) script: Mutex<Script>,
    calls: AtomicUsize,
}

impl SearchRelay {
    fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script),
            calls: AtomicUsize::new(0),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RelayTransport for SearchRelay {
    fn fetch<'a>(
        &'a self,
        _request: &'a call_relay::Request,
    ) -> RelayFuture<'a, call_relay::Response> {
        Box::pin(async { Err(call_relay::Error::Disabled) })
    }

    fn call_method_rotated<'a>(
        &'a self,
        method_id: &'a str,
        _script: &'a str,
        _inputs: Bytes,
        _region_rotation: i32,
    ) -> RelayFuture<'a, Bytes> {
        assert_eq!(
            method_id, "sc.search",
            "live search only ever runs the search method"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        let script = self.script.lock().unwrap().clone();
        Box::pin(async move {
            match script {
                Script::Answers(items) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(Bytes::from(
                        json!({"ok": true, "collection": items, "next_href": null}).to_string(),
                    ))
                }
                Script::Slow(takes) => {
                    tokio::time::sleep(takes).await;
                    Err(call_relay::Error::Disabled)
                }
                Script::Silent => Err(call_relay::Error::Disabled),
            }
        })
    }
}

pub(super) struct Harness {
    pub(super) live: Arc<LiveSearch>,
    pub(super) relay: Arc<SearchRelay>,
    pub(super) tag: String,
}

pub(super) fn redis() -> deadpool_redis::Pool {
    deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))
    .expect("a redis pool builds without connecting")
}

fn live_admission() -> Arc<PublicAdmission> {
    let open = AdmissionLimitCfg {
        per_client: 100_000,
        global: 100_000,
    };
    let shut = AdmissionLimitCfg {
        per_client: 0,
        global: 0,
    };
    PublicAdmission::for_live_search(
        redis(),
        AdmissionCfg {
            window: Duration::from_secs(60),
            timeout: Duration::from_millis(500),
            max_in_flight: 256,
            login: open,
            link_create: open,
            resolve: open,
            live_main: open,
            live_side: open,
            live_import: open,
            live_rescue: open,
            live_proxy: shut,
            live_entity: open,
        },
    )
}

fn harness(pool: &PgPool, script: Script, mode: LiveMode) -> anyhow::Result<Harness> {
    harness_with(
        pool,
        script,
        LiveSearchCfg {
            mode,
            db_rescue: false,
            max_in_flight: 8,
            ranked: false,
        },
    )
}

pub(super) fn harness_with(
    pool: &PgPool,
    script: Script,
    cfg: LiveSearchCfg,
) -> anyhow::Result<Harness> {
    harness_on(pool, script, cfg, redis())
}

fn harness_on(
    pool: &PgPool,
    script: Script,
    cfg: LiveSearchCfg,
    cache: deadpool_redis::Pool,
) -> anyhow::Result<Harness> {
    let relay = SearchRelay::new(script);
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: String::new(),
        proxy_fallback: false,
        api_base: Some("http://127.0.0.1:1".to_owned()),
        home_base: Some("http://127.0.0.1:1".to_owned()),
    })?
    .with_relay(relay.clone());
    let auth = AuthService::new(
        pool.clone(),
        sc.clone(),
        OAuthAppsService::new(pool.clone()),
        AuthHealthService::with_database(redis(), pool.clone()),
    );
    let tokens = TokenProvider::new(auth, OAuthAppTokenService::new(pool.clone()));
    let read = ScReadService::new(sc, tokens, pool.clone());
    let live = LiveSearch::new(
        read,
        CacheService::new(cache),
        live_admission(),
        pool.clone(),
        cfg,
    );
    Ok(Harness {
        live,
        relay,
        tag: uuid::Uuid::now_v7().simple().to_string(),
    })
}

pub(super) fn hits(base: u64, n: u64) -> Vec<Value> {
    (base..base + n)
        .map(|id| {
            json!({
                "id": id,
                "kind": "track",
                "urn": format!("soundcloud:tracks:{id}"),
                "title": format!("Live hit {id}"),
                "policy": "ALLOW",
                "duration": 180000,
                "user": {"id": 7, "kind": "user", "urn": "soundcloud:users:7", "username": "uploader"}
            })
        })
        .collect()
}

pub(super) fn base() -> u64 {
    9_000_000_000 + u64::from(uuid::Uuid::now_v7().as_fields().1) * 1000
}

pub(super) fn rows(base: u64, n: u64, title: &str) -> Vec<Value> {
    (base..base + n)
        .map(|id| json!({"urn": format!("soundcloud:tracks:{id}"), "title": title, "user": {"username": "someone"}}))
        .collect()
}

fn page_of(collection: Vec<Value>, page: i64, has_more: bool) -> AppResult<ListPageResult<Value>> {
    Ok(ListPageResult {
        collection,
        page,
        page_size: 20,
        has_more,
    })
}

impl Harness {
    pub(super) fn phrase(&self, words: &str) -> String {
        format!("{words} {}", self.tag)
    }

    pub(super) fn request(
        &self,
        kind: LiveKind,
        phrase: &str,
        intent: Option<&'static str>,
        page: Option<i64>,
        limit: i64,
        linked: bool,
    ) -> LiveRequest {
        let mut headers = HeaderMap::new();
        if let Some(intent) = intent {
            headers.insert("x-search-intent", HeaderValue::from_static(intent));
        }
        self.live
            .plan(
                kind,
                Some(phrase),
                &headers,
                &PaginationQuery {
                    page,
                    limit: Some(limit),
                },
                linked,
                &format!("listener-{}", self.tag),
            )
            .expect("the phrase is eligible")
    }

    async fn tracks(&self, phrase: &str, page: i64, local: Vec<Value>) -> LivePage {
        let request = self.request(LiveKind::Tracks, phrase, Some("sc"), Some(page), 20, false);
        self.ask(&request, local, Duration::ZERO).await
    }

    pub(super) async fn ask(
        &self,
        request: &LiveRequest,
        local: Vec<Value>,
        takes: Duration,
    ) -> LivePage {
        self.live
            .page(request, |page| {
                let local = local.clone();
                async move {
                    tokio::time::sleep(takes).await;
                    if page == 0 {
                        page_of(local, 0, false)
                    } else {
                        page_of(Vec::new(), page, false)
                    }
                }
            })
            .await
            .expect("live search never turns a SoundCloud failure into an error")
    }
}

pub(super) fn sources(page: &LivePage) -> Vec<&str> {
    page.page
        .collection
        .iter()
        .map(|item| item["_scd_search"]["source"].as_str().unwrap_or("untagged"))
        .collect()
}

pub(super) fn titles(page: &LivePage) -> Vec<String> {
    page.page
        .collection
        .iter()
        .map(|item| item["title"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_repeated_query_is_answered_from_the_window_without_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness(&pool, Script::Answers(hits(at, 3)), LiveMode::Explicit)?;
    let phrase = lab.phrase("lucid dreams");

    let first = lab.tracks(&phrase, 0, Vec::new()).await;
    assert_eq!(first.state(), LiveState::Fresh);
    assert_eq!(first.page.collection.len(), 3);
    assert_eq!(sources(&first), ["soundcloud"; 3]);
    assert_eq!(first.page.collection[0]["access"], "playable");

    let second = lab.tracks(&phrase, 0, Vec::new()).await;
    assert_eq!(second.state(), LiveState::Cached);
    assert_eq!(titles(&second), titles(&first));
    assert_eq!(
        lab.relay.calls(),
        1,
        "the second ask must not reach SoundCloud"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_hundred_listeners_asking_at_once_cost_one_search(pool: PgPool) -> anyhow::Result<()> {
    let lab = harness(&pool, Script::Answers(hits(base(), 5)), LiveMode::Explicit)?;
    let phrase = lab.phrase("kavinsky nightcall");

    let pages =
        futures::future::join_all((0..100).map(|_| lab.tracks(&phrase, 0, Vec::new()))).await;

    assert_eq!(lab.relay.calls(), 1);
    assert!(pages.iter().all(|page| page.page.collection.len() == 5
        && matches!(page.state(), LiveState::Fresh | LiveState::Cached)));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_relay_that_answers_nobody_opens_the_breaker_and_is_left_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness(&pool, Script::Silent, LiveMode::Explicit)?;
    let local = rows(base(), 2, "local only");

    for round in 0..4 {
        let page = lab
            .tracks(&lab.phrase(&format!("silent {round}")), 0, local.clone())
            .await;
        assert_eq!(page.state(), LiveState::Unavailable);
        assert_eq!(
            sources(&page),
            ["local", "local"],
            "local rows still answer"
        );
    }
    assert_eq!(lab.relay.calls(), 4);

    let cooled = lab.tracks(&lab.phrase("silent 5"), 0, local).await;
    assert_eq!(cooled.state(), LiveState::Cooling);
    assert_eq!(
        lab.relay.calls(),
        4,
        "an open breaker stops every SoundCloud call"
    );

    let channels: Vec<String> = sqlx::query_scalar("SELECT channel FROM sc_egress_health")
        .fetch_all(&pool)
        .await?;
    assert_eq!(
        channels,
        ["search_live"],
        "search trips its own breaker and never writes relay_lua"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_failing_flight_is_shared_by_sixteen_waiters_and_then_remembered(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness(&pool, Script::Silent, LiveMode::Explicit)?;
    let phrase = lab.phrase("nobody answers");

    let pages =
        futures::future::join_all((0..16).map(|_| lab.tracks(&phrase, 0, Vec::new()))).await;
    assert_eq!(lab.relay.calls(), 1);
    assert!(
        pages
            .iter()
            .all(|page| page.state() == LiveState::Unavailable)
    );

    let again = lab.tracks(&phrase, 0, Vec::new()).await;
    assert_eq!(again.state(), LiveState::Unavailable);
    assert!(
        again
            .live
            .retry_after_sec
            .is_some_and(|left| left > 0 && left <= 60)
    );
    assert_eq!(
        lab.relay.calls(),
        1,
        "the fail key holds the query for a minute"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_local_page_gets_six_hundred_ms_before_soundcloud_is_asked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness(&pool, Script::Answers(hits(at, 2)), LiveMode::Explicit)?;
    let phrase = lab.phrase("enough here");
    let mut enough = rows(at + 500, 9, "filler");
    enough.push(json!({"urn": format!("soundcloud:tracks:{}", at + 600), "title": phrase, "user": {"username": "x"}}));

    let request = lab.request(LiveKind::Tracks, &phrase, Some("sc"), Some(0), 20, false);
    let quick = lab
        .ask(&request, enough.clone(), Duration::from_millis(100))
        .await;
    assert_eq!(quick.state(), LiveState::Local);
    assert_eq!(quick.page.collection.len(), 10);
    assert_eq!(lab.relay.calls(), 0);

    let late = lab.ask(&request, enough, Duration::from_millis(900)).await;
    assert_eq!(late.state(), LiveState::Fresh);
    assert_eq!(
        lab.relay.calls(),
        1,
        "a local page slower than 600 ms no longer holds live back"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_slow_relay_is_cut_at_its_wait_and_the_page_falls_back_to_local(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness(&pool, Script::Slow(Duration::from_secs(10)), LiveMode::Auto)?;
    let phrase = lab.phrase("slow relay");
    let request = lab.request(LiveKind::Tracks, &phrase, Some("fill"), Some(0), 20, false);
    assert_eq!(request.class, LiveClass::Fill);

    let started = Instant::now();
    let page = lab
        .ask(&request, rows(base(), 1, "local"), Duration::ZERO)
        .await;
    let took = started.elapsed();

    assert_eq!(page.state(), LiveState::Timeout);
    assert_eq!(sources(&page), ["local"]);
    assert!(
        took >= Duration::from_millis(1900) && took < Duration::from_millis(2600),
        "the fill class waits 2 s for the relay inside its 2.5 s budget, took {took:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_pause_row_stops_live_search_and_keeps_cached_windows(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness(&pool, Script::Answers(hits(base(), 4)), LiveMode::Explicit)?;
    let cached = lab.phrase("cached before the pause");
    assert_eq!(
        lab.tracks(&cached, 0, Vec::new()).await.state(),
        LiveState::Fresh
    );

    sqlx::query(
        "INSERT INTO sc_egress_health (channel, open_until, opened_by)
         VALUES ('search_live_pause', now() + interval '6 hours', 'ops')",
    )
    .execute(&pool)
    .await?;
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let paused = lab
        .tracks(
            &lab.phrase("asked during the pause"),
            0,
            rows(base(), 1, "local"),
        )
        .await;
    assert_eq!(paused.state(), LiveState::Paused);
    assert!(
        paused
            .live
            .retry_after_sec
            .is_some_and(|left| left > 6 * 3600 - 60 && left <= 6 * 3600),
        "a pause reports how long it still holds: {:?}",
        paused.live.retry_after_sec
    );
    let kept = lab.tracks(&cached, 0, Vec::new()).await;
    assert_eq!(kept.state(), LiveState::Cached);
    assert_eq!(kept.page.collection.len(), 4);
    assert_eq!(lab.relay.calls(), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn later_pages_read_the_window_and_then_continue_with_local_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness(&pool, Script::Answers(hits(at, 40)), LiveMode::Explicit)?;
    let phrase = lab.phrase("forty hits");
    let mut local = rows(at + 500, 3, "local row");
    local.push(json!({"urn": format!("soundcloud:tracks:{at}"), "title": "duplicate of a hit"}));

    let first = lab.tracks(&phrase, 0, local.clone()).await;
    assert_eq!(first.page.collection.len(), 20);
    assert!(first.page.has_more);

    let second = lab.tracks(&phrase, 1, local.clone()).await;
    assert_eq!(second.state(), LiveState::Cached);
    assert_eq!(second.page.collection.len(), 20);
    assert_eq!(
        second.page.collection[0]["title"],
        format!("Live hit {}", at + 20)
    );
    assert!(second.page.has_more, "local rows follow the window");

    let third = lab.tracks(&phrase, 2, local).await;
    assert_eq!(third.page.page, 2);
    assert_eq!(
        titles(&third),
        ["local row"; 3],
        "page 2 is the local page 0 without the rows the window already showed"
    );
    assert_eq!(sources(&third), ["local"; 3]);
    assert_eq!(
        lab.relay.calls(),
        1,
        "pages after the first never reach SoundCloud"
    );

    let unseen = lab
        .tracks(&lab.phrase("never searched"), 1, Vec::new())
        .await;
    assert_eq!(unseen.state(), LiveState::Skipped);
    Ok(())
}

fn enough_for(phrase: &str, at: u64) -> Vec<Value> {
    let mut enough = rows(at, 9, "filler");
    enough.push(json!({"urn": format!("soundcloud:tracks:{}", at + 100), "title": phrase, "user": {"username": "x"}}));
    enough
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_local_page_runs_while_the_window_is_read(pool: PgPool) -> anyhow::Result<()> {
    let silent = std::net::TcpListener::bind("127.0.0.1:0")?;
    let unanswered = deadpool_redis::Config::from_url(format!("redis://{}", silent.local_addr()?))
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let lab = harness_on(
        &pool,
        Script::Answers(hits(base(), 2)),
        LiveSearchCfg {
            mode: LiveMode::Explicit,
            db_rescue: false,
            max_in_flight: 8,
            ranked: false,
        },
        unanswered,
    )?;
    let phrase = lab.phrase("slow window");
    let request = lab.request(LiveKind::Tracks, &phrase, Some("sc"), Some(0), 20, false);

    let started = Instant::now();
    let page = lab
        .ask(
            &request,
            enough_for(&phrase, base()),
            Duration::from_millis(300),
        )
        .await;

    assert_eq!(page.state(), LiveState::Local);
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "a window read stuck for 400 ms must not delay a 300 ms local page, took {:?}",
        started.elapsed()
    );
    assert_eq!(lab.relay.calls(), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_stale_window_never_follows_a_first_page_that_went_local(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness(&pool, Script::Answers(hits(at, 40)), LiveMode::Explicit)?;
    let phrase = lab.phrase("stale window");
    let request = lab.request(LiveKind::Tracks, &phrase, Some("sc"), Some(0), 20, false);
    let stale_ids: Vec<String> = (at + 1000..at + 1040)
        .map(|id| format!("soundcloud:tracks:{id}"))
        .collect();
    LiveStore::new(CacheService::new(redis()))
        .write(
            request.class.scope(request.kind),
            &request.query.hash,
            &Window::new(stale_ids, chrono::Utc::now().timestamp() - 700),
            1800,
            &hits(at + 1000, 40),
            &[],
        )
        .await;
    let enough = enough_for(&phrase, at + 500);

    let first = lab.ask(&request, enough.clone(), Duration::ZERO).await;
    assert_eq!(first.state(), LiveState::Local);

    let later = lab.request(LiveKind::Tracks, &phrase, Some("sc"), Some(1), 20, false);
    let second = lab.ask(&later, enough, Duration::ZERO).await;
    assert_eq!(second.state(), LiveState::Skipped);
    assert_eq!(second.page.page, 1);
    assert!(
        second.page.collection.is_empty(),
        "page 1 continues the local pages page 0 started, not the old window: {:?}",
        titles(&second)
    );
    assert_eq!(lab.relay.calls(), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_import_puts_the_soundcloud_top_hit_first(pool: PgPool) -> anyhow::Result<()> {
    let at = base();
    let lab = harness(&pool, Script::Answers(hits(at, 10)), LiveMode::Explicit)?;
    let phrase = lab.phrase("artist title");
    let request = lab.request(LiveKind::Tracks, &phrase, None, None, 3, true);
    assert_eq!(request.class, LiveClass::Import);

    let page = lab
        .ask(&request, rows(at + 500, 3, "local"), Duration::ZERO)
        .await;

    assert_eq!(page.page.collection.len(), 3);
    assert_eq!(
        page.page.collection[0]["urn"],
        format!("soundcloud:tracks:{at}")
    );
    assert_eq!(sources(&page), ["soundcloud"; 3]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_fill_is_off_in_explicit_mode_and_a_failed_local_page_keeps_the_window(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness(&pool, Script::Answers(hits(base(), 2)), LiveMode::Explicit)?;
    let phrase = lab.phrase("explicit only");
    let fill = lab.request(LiveKind::Tracks, &phrase, Some("fill"), Some(0), 20, false);
    let off = lab
        .ask(&fill, rows(base(), 1, "local"), Duration::ZERO)
        .await;
    assert_eq!(off.state(), LiveState::Off);
    assert_eq!(lab.relay.calls(), 0);

    let main = lab.request(LiveKind::Tracks, &phrase, Some("sc"), Some(0), 20, false);
    let page = lab
        .live
        .page(&main, |_| async {
            Err(crate::modules::search::failure::timed_out())
        })
        .await?;
    assert_eq!(page.state(), LiveState::Fresh);
    assert_eq!(page.live.local, Some("unavailable"));
    assert_eq!(page.page.collection.len(), 2);
    assert!(!page.page.has_more);
    Ok(())
}

fn rescuing(mode: LiveMode) -> LiveSearchCfg {
    LiveSearchCfg {
        mode,
        db_rescue: true,
        max_in_flight: 8,
        ranked: false,
    }
}

fn first_page() -> PaginationQuery {
    PaginationQuery {
        page: Some(0),
        limit: Some(20),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn only_an_old_client_on_page_zero_with_a_specific_phrase_is_rescued(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness_with(&pool, Script::Silent, rescuing(LiveMode::Auto))?;
    let none = HeaderMap::new();
    let mut wall = HeaderMap::new();
    wall.insert("x-search-intent", HeaderValue::from_static("wall"));
    let later = PaginationQuery {
        page: Some(1),
        limit: Some(20),
    };

    let rescue = lab
        .live
        .rescue_plan(Some("lucid dreams"), &none, &first_page(), "17")
        .expect("an old default-mode search is rescued");
    assert_eq!(rescue.class, LiveClass::Rescue);
    assert_eq!(rescue.kind, LiveKind::Tracks);
    assert!(
        lab.live
            .rescue_plan(Some("lucid dreams"), &wall, &first_page(), "17")
            .is_none(),
        "a client that names its intent opted out"
    );
    assert!(
        lab.live
            .rescue_plan(Some("lucid dreams"), &none, &later, "17")
            .is_none()
    );
    assert!(
        lab.live
            .rescue_plan(Some("abc"), &none, &first_page(), "17")
            .is_none(),
        "a short single word is too vague to spend SoundCloud on"
    );
    assert!(
        lab.live
            .rescue_plan(None, &none, &first_page(), "17")
            .is_none()
    );

    for cfg in [
        rescuing(LiveMode::Explicit),
        LiveSearchCfg {
            mode: LiveMode::Auto,
            db_rescue: false,
            max_in_flight: 8,
            ranked: false,
        },
    ] {
        let off = harness_with(&pool, Script::Silent, cfg)?;
        assert!(
            off.live
                .rescue_plan(Some("lucid dreams"), &none, &first_page(), "17")
                .is_none()
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_empty_wall_page_is_rescued_with_the_soundcloud_window(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness_with(
        &pool,
        Script::Answers(hits(at, 3)),
        rescuing(LiveMode::Auto),
    )?;
    let phrase = lab.phrase("nothing local");
    let request = lab
        .live
        .rescue_plan(Some(&phrase), &HeaderMap::new(), &first_page(), "17")
        .expect("eligible");

    let page = lab
        .live
        .page(&request, |_| async { page_of(Vec::new(), 0, false) })
        .await?;

    assert_eq!(page.state(), LiveState::Fresh);
    assert_eq!(sources(&page), ["soundcloud"; 3]);
    assert_eq!(lab.relay.calls(), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn substitution_hides_deleted_rows_and_folds_copies_into_their_winner(
    pool: PgPool,
) -> anyhow::Result<()> {
    for (id, sharing, deleted) in [
        ("1", "public", false),
        ("2", "public", true),
        ("3", "public", false),
        ("4", "public", false),
        ("5", "private", false),
    ] {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, deleted_at)
             VALUES ($1, 'soundcloud:tracks:' || $1, 'Local ' || $1, 'local ' || $1, 180000, $2,
                     CASE WHEN $3 THEN now() END)",
        )
        .bind(id)
        .bind(sharing)
        .bind(deleted)
        .execute(&pool)
        .await?;
    }
    sqlx::query(
        "UPDATE tracks SET superseded_by = (SELECT id FROM tracks WHERE sc_track_id = '4')
         WHERE sc_track_id = '3'",
    )
    .execute(&pool)
    .await?;

    let urns: Vec<String> = (1..=6)
        .map(|id| format!("soundcloud:tracks:{id}"))
        .collect();
    let serving = serving::lookup(&pool, LiveKind::Tracks, &urns).await?;

    assert_eq!(
        serving["soundcloud:tracks:1"]
            .as_ref()
            .map(|t| t["title"].clone()),
        Some(json!("Local 1"))
    );
    assert_eq!(
        serving["soundcloud:tracks:2"], None,
        "a deleted row drops its hit"
    );
    assert_eq!(
        serving["soundcloud:tracks:3"]
            .as_ref()
            .map(|t| t["urn"].clone()),
        Some(json!("soundcloud:tracks:4")),
        "a superseded copy is served as its winner"
    );
    assert_eq!(
        serving["soundcloud:tracks:5"], None,
        "a private row drops its hit"
    );
    assert!(
        !serving.contains_key("soundcloud:tracks:6"),
        "a hit we do not know stays a live hit"
    );
    Ok(())
}
