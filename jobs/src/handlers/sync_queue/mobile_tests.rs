use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::SyncQueueHandler;
use super::mobile::MobileSender;
use super::unclear_create_tests::{handler, run};

const USER: &str = "17";
const TRACK: &str = "soundcloud:tracks:42";
const MOMENT: &str = "soundcloud:tracks:42:moments:5";
const PERSON: &str = "soundcloud:users:9";
const COMMENT: &str = "soundcloud:comments:5";
const CAPTCHA_PAGE: &str = r#"{"url":"https://geo.captcha-delivery.com/interstitial/?cid=1"}"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Missing,
    Active,
    RelinkRequired,
}

#[derive(Clone)]
enum Canned {
    Status(u16),
    Json(u16, Value),
    Captcha,
    Plan(u16, &'static str),
}

struct Seen {
    method: String,
    target: String,
    body: String,
    authorization: Option<String>,
}

#[derive(Clone, Default)]
struct FakeMobile {
    seen: Arc<Mutex<Vec<Seen>>>,
    replies: Arc<Mutex<VecDeque<Canned>>>,
    tokens: Arc<AtomicUsize>,
}

impl FakeMobile {
    fn reply_with(&self, replies: impl IntoIterator<Item = Canned>) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(replies);
    }

    fn seen(&self) -> Vec<(String, String)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|seen| (seen.method.clone(), seen.target.clone()))
            .collect()
    }

    fn sent(&self, index: usize) -> Option<(String, Option<String>)> {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.get(index)
            .map(|seen| (seen.body.clone(), seen.authorization.clone()))
    }

    fn count(&self) -> usize {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

async fn mobile_answer(
    State(fake): State<FakeMobile>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let target = headers
        .get("x-target")
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_default();
    if target == sc_mobile::TOKEN_URL {
        let call = fake.tokens.fetch_add(1, Ordering::SeqCst) + 1;
        return Json(json!({
            "access_token": format!("fresh-access-{call}"),
            "refresh_token": format!("fresh-refresh-{call}"),
            "expires_in": 604799,
        }))
        .into_response();
    }
    fake.seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Seen {
            method: method.to_string(),
            target,
            body: String::from_utf8_lossy(&body).into_owned(),
            authorization: headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        });
    let canned = fake
        .replies
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .unwrap_or(Canned::Status(200));
    match canned {
        Canned::Status(status) => status_of(status).into_response(),
        Canned::Json(status, body) => (status_of(status), Json(body)).into_response(),
        Canned::Captcha => (StatusCode::FORBIDDEN, CAPTCHA_PAGE).into_response(),
        Canned::Plan(status, plan) => {
            (status_of(status), [("SC-Mob-UserPlan", plan)]).into_response()
        }
    }
}

fn status_of(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn serve_mobile(fake: FakeMobile) -> anyhow::Result<SocketAddr> {
    let app = Router::new().fallback(mobile_answer).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(address)
}

async fn serve_apiv1(hits: Arc<AtomicUsize>) -> anyhow::Result<SocketAddr> {
    let app = Router::new().fallback(move || {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            Json(json!({}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(address)
}

struct World {
    pool: PgPool,
    handler: SyncQueueHandler,
    mobile: FakeMobile,
    apiv1: Arc<AtomicUsize>,
}

async fn world(pool: PgPool, profile: Profile) -> anyhow::Result<World> {
    crate::db::migrations::run_core(&pool, None).await?;
    super::track_tests::connect_owner(&pool).await?;
    if profile != Profile::Missing {
        link(&pool, profile).await?;
    }
    let mobile = FakeMobile::default();
    let proxy = serve_mobile(mobile.clone()).await?;
    let apiv1 = Arc::new(AtomicUsize::new(0));
    let mut handler = handler(&pool, serve_apiv1(apiv1.clone()).await?)?;
    handler.mobile = MobileSender::new(pool.clone(), Some(format!("http://{proxy}/").parse()?));
    Ok(World {
        pool,
        handler,
        mobile,
        apiv1,
    })
}

async fn link(pool: &PgPool, profile: Profile) -> anyhow::Result<()> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (
             id, name, client_id, client_secret, redirect_uri,
             client_kind, app_id, app_version, user_agent
         ) VALUES (
             $1, 'mobile', 'fake-client', 'fake-secret', 'https://app.test/callback',
             'mobile', 'fake.app', '1.0.0', 'FakeAgent/1.0'
         )",
    )
    .bind(app)
    .execute(pool)
    .await?;
    let status = if profile == Profile::RelinkRequired {
        "relink_required"
    } else {
        "active"
    };
    sqlx::query(
        "INSERT INTO mobile_credentials (
             soundcloud_user_id, oauth_app_id, device_id, access_token, refresh_token,
             expires_at, scope, linked_at, status
         ) VALUES ($1, $2, $3, 'old-access', 'old-refresh', now() + interval '6 days', '*', now(), $4)",
    )
    .bind(USER)
    .bind(app)
    .bind(Uuid::new_v4())
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

impl World {
    async fn queue(
        &self,
        action: &str,
        target: &str,
        payload: Option<Value>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO sync_queue (user_id, action_type, target_urn, payload)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(USER)
        .bind(action)
        .bind(target)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn send(&self, action: &str, target: &str, payload: Option<Value>) -> anyhow::Result<()> {
        self.queue(action, target, payload).await?;
        run(&self.pool, &self.handler).await
    }

    async fn queued(&self) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM sync_queue")
            .fetch_one(&self.pool)
            .await?)
    }

    async fn row(&self) -> anyhow::Result<(i32, bool, Option<String>)> {
        Ok(
            sqlx::query_as("SELECT retry_count, dead, last_error FROM sync_queue")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    async fn credential(&self) -> anyhow::Result<(String, String, Option<String>)> {
        Ok(sqlx::query_as(
            "SELECT status, access_token, plan FROM mobile_credentials WHERE soundcloud_user_id = $1",
        )
        .bind(USER)
        .fetch_one(&self.pool)
        .await?)
    }

    fn apiv1_hits(&self) -> usize {
        self.apiv1.load(Ordering::SeqCst)
    }
}

fn reaction_payload() -> Option<Value> {
    None
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_follow_goes_through_mobile_and_skips_apiv1(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Status(201)]);

    world.send("follow_user", PERSON, None).await?;

    let seen = world.mobile.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "POST");
    assert!(seen[0].1.contains("/follows/users/soundcloud:users:9"));
    assert_eq!(
        world.mobile.sent(0).and_then(|sent| sent.1).as_deref(),
        Some("OAuth old-access")
    );
    assert_eq!(world.apiv1_hits(), 0);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_unfollow_is_a_mobile_delete(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Status(200)]);

    world.send("unfollow_user", PERSON, None).await?;

    let seen = world.mobile.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "DELETE");
    assert!(seen[0].1.contains("/follows/users/soundcloud:users:9"));
    assert_eq!(world.apiv1_hits(), 0);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unlinked_follow_keeps_the_apiv1_path_and_never_touches_mobile(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world.send("follow_user", PERSON, None).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.mobile.tokens.load(Ordering::SeqCst), 0);
    assert_eq!(world.apiv1_hits(), 1);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_like_stays_on_apiv1_for_a_linked_user(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;

    world.send("like_track", TRACK, None).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.apiv1_hits(), 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bot_check_on_a_follow_is_not_retried_and_the_follow_goes_to_apiv1(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Captcha, Canned::Status(201)]);

    world.send("follow_user", PERSON, None).await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1_hits(), 1);
    assert_eq!(world.queued().await?, 0);
    assert_eq!(world.credential().await?.0, "active");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_profile_that_needs_relinking_sends_a_follow_to_apiv1_without_a_mobile_call(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::RelinkRequired).await?;

    world.send("follow_user", PERSON, None).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.apiv1_hits(), 1);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_401_is_refreshed_once_and_a_second_401_marks_relink_required_and_falls_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Status(401), Canned::Status(401)]);

    world.send("follow_user", PERSON, None).await?;

    assert_eq!(world.mobile.count(), 2);
    assert_eq!(world.mobile.tokens.load(Ordering::SeqCst), 1);
    assert_eq!(
        world.mobile.sent(1).and_then(|sent| sent.1).as_deref(),
        Some("OAuth fresh-access-1")
    );
    let (status, _, _) = world.credential().await?;
    assert_eq!(status, "relink_required");
    assert_eq!(world.apiv1_hits(), 1);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_401_that_a_refresh_fixes_finishes_the_write_on_mobile(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Status(401), Canned::Status(201)]);

    world.send("follow_user", PERSON, None).await?;

    assert_eq!(world.mobile.count(), 2);
    assert_eq!(world.apiv1_hits(), 0);
    let (status, token, _) = world.credential().await?;
    assert_eq!(
        (status.as_str(), token.as_str()),
        ("active", "fresh-access-1")
    );
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_429_on_a_mobile_follow_holds_the_account_action_and_does_not_fall_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Status(429)]);

    world.send("follow_user", PERSON, None).await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1_hits(), 0);
    assert_eq!(world.queued().await?, 1);
    assert!(
        world
            .handler
            .account_pauses
            .remaining_seconds(&(USER.to_owned(), "follow_user".to_owned()))
            .is_some()
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_reaction_without_a_linked_profile_fails_with_a_reason_and_sends_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world
        .send("track_reaction_set", MOMENT, reaction_payload())
        .await?;

    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_not_linked".to_owned()))
    );
    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.apiv1_hits(), 0);
    Ok(())
}

const SYNC_MAX: i32 = backend_contracts::SYNC_QUEUE_MAX_RETRIES;

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_reaction_on_a_profile_that_needs_relinking_fails_with_its_reason(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::RelinkRequired).await?;

    world
        .send("track_reaction_remove", MOMENT, reaction_payload())
        .await?;

    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_relink_required".to_owned()))
    );
    assert_eq!(world.mobile.count(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_reaction_is_a_graphql_upsert_and_a_removal_is_a_graphql_remove(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([
        Canned::Json(
            200,
            json!({"data": {"upsertInteraction": {"targetUrn": "ts:5"}}}),
        ),
        Canned::Json(200, json!({"data": {"removeInteraction": true}})),
    ]);

    world
        .send("track_reaction_set", MOMENT, reaction_payload())
        .await?;
    world
        .send("track_reaction_remove", MOMENT, reaction_payload())
        .await?;

    let seen = world.mobile.seen();
    assert_eq!(seen.len(), 2);
    for (method, target) in &seen {
        assert_eq!(method, "POST");
        assert!(target.starts_with("https://graph.soundcloud.com/graphql"));
    }
    let bodies: Vec<Value> = (0..2)
        .filter_map(|index| world.mobile.sent(index))
        .filter_map(|(body, _)| serde_json::from_str(&body).ok())
        .collect();
    assert!(
        bodies[0]["query"]
            .as_str()
            .is_some_and(|query| query.contains("upsertInteraction"))
    );
    assert!(
        bodies[1]["query"]
            .as_str()
            .is_some_and(|query| query.contains("removeInteraction"))
    );
    for body in &bodies {
        assert_eq!(
            body["variables"]["input"],
            json!({
                "targetUrn": "ts:5",
                "parentUrn": TRACK,
                "interactionTypeUrn": "sc:interactiontype:trackreaction",
                "interactionTypeValueUrn": "sc:interactiontypevalue:1f525",
            })
        );
    }
    assert_eq!(world.apiv1_hits(), 0);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn graphql_errors_on_a_reaction_are_a_failure_that_is_retried_later(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Json(
        200,
        json!({"errors": [{"message": "nope"}], "data": null}),
    )]);

    world
        .send("track_reaction_set", MOMENT, reaction_payload())
        .await?;

    let (retries, dead, _) = world.row().await?;
    assert_eq!((retries, dead), (1, false));
    assert_eq!(world.apiv1_hits(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bot_check_on_a_reaction_fails_it_without_a_retry_or_an_apiv1_call(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world
        .send("track_reaction_set", MOMENT, reaction_payload())
        .await?;

    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_bot_check".to_owned()))
    );
    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1_hits(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_reaction_with_a_bad_payload_is_parked_before_anything_is_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;

    world
        .send("track_reaction_set", MOMENT, Some(json!({"value_urn": 5})))
        .await?;

    assert!(world.row().await?.1);
    assert_eq!(world.mobile.count(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_comment_delete_is_a_mobile_rest_delete_and_a_gone_comment_closes_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Status(204), Canned::Status(404)]);

    world.send("comment_delete", COMMENT, None).await?;
    assert_eq!(world.queued().await?, 0);
    world
        .send("comment_delete", "soundcloud:comments:6", None)
        .await?;
    assert_eq!(world.queued().await?, 0);

    let seen = world.mobile.seen();
    assert_eq!(seen[0].0, "DELETE");
    assert!(seen[0].1.contains("/comments/soundcloud:comments:5"));
    assert_eq!(seen.len(), 2);
    assert_eq!(world.apiv1_hits(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_comment_delete_without_a_linked_profile_fails_and_never_goes_to_apiv1(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world.send("comment_delete", COMMENT, None).await?;

    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_not_linked".to_owned()))
    );
    assert_eq!(world.apiv1_hits(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_plan_header_is_written_only_when_it_changes(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([
        Canned::Plan(201, "high_tier"),
        Canned::Plan(201, "high_tier"),
        Canned::Plan(201, "none"),
    ]);
    let seen_at = |world: &World| {
        let pool = world.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
                "SELECT plan_seen_at FROM mobile_credentials",
            )
            .fetch_one(&pool)
            .await
        }
    };

    world.send("follow_user", PERSON, None).await?;
    let first = seen_at(&world).await?;
    assert_eq!(world.credential().await?.2.as_deref(), Some("high_tier"));

    world
        .send("follow_user", "soundcloud:users:10", None)
        .await?;
    assert_eq!(seen_at(&world).await?, first);

    world
        .send("follow_user", "soundcloud:users:11", None)
        .await?;
    assert_eq!(world.credential().await?.2.as_deref(), Some("none"));
    assert_ne!(seen_at(&world).await?, first);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn after_a_bot_check_follows_skip_mobile_for_the_account_action(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world.send("follow_user", PERSON, None).await?;
    world
        .send("follow_user", "soundcloud:users:10", None)
        .await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1_hits(), 2);
    assert_eq!(world.queued().await?, 0);
    world.mobile.reply_with([Canned::Status(200)]);
    world.send("unfollow_user", PERSON, None).await?;
    assert_eq!(world.mobile.count(), 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn after_a_bot_check_a_reaction_fails_without_sending(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world.send("track_reaction_set", MOMENT, None).await?;
    sqlx::query("DELETE FROM sync_queue")
        .execute(&world.pool)
        .await?;
    world
        .send("track_reaction_set", "soundcloud:tracks:42:moments:6", None)
        .await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_bot_check".to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn reactions_at_other_seconds_are_other_writes_and_one_moment_is_one_write(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    for (action, target) in [
        ("track_reaction_set", "soundcloud:tracks:42:moments:5"),
        ("track_reaction_set", "soundcloud:tracks:42:moments:5"),
        ("track_reaction_set", "soundcloud:tracks:42:moments:6"),
    ] {
        sqlx::query_file!(
            "../api/queries/sync_queue/service/enqueue.sql",
            USER,
            action,
            target,
            &Value::Null,
            false
        )
        .execute(&world.pool)
        .await?;
    }
    assert_eq!(world.queued().await?, 2);

    world
        .queue(
            "track_reaction_remove",
            "soundcloud:tracks:42:moments:5",
            None,
        )
        .await?;
    sqlx::query("UPDATE sync_queue SET next_run_at = now()")
        .execute(&world.pool)
        .await?;
    world.handler.repository.adopt().await?;

    let mut claimed: Vec<(String, String)> = world
        .handler
        .repository
        .claim(10)
        .await?
        .into_iter()
        .map(|mutation| (mutation.action_type, mutation.target_urn))
        .collect();
    claimed.sort();

    assert_eq!(
        claimed,
        [
            (
                "track_reaction_set".to_owned(),
                "soundcloud:tracks:42:moments:5".to_owned()
            ),
            (
                "track_reaction_set".to_owned(),
                "soundcloud:tracks:42:moments:6".to_owned()
            ),
        ]
    );
    Ok(())
}
