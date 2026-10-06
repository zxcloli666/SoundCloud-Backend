use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;

use super::entity_miss::EntityMiss;
use super::service_tests::redis;
use super::stash::LiveStash;
use super::stash_tests::indexing;
use crate::cache::CacheService;
use crate::common::admission::PublicAdmission;
use crate::config::{AdmissionCfg, AdmissionLimitCfg};
use crate::modules::auth::{AuthHealthService, AuthService, TokenProvider};
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::sc::{ScClient, ScReadService};
use sc_transport::{Bytes, RelayTransport};

type RelayFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, call_relay::Error>> + Send + 'a>>;

struct EntityRelay {
    calls: AtomicUsize,
}

impl RelayTransport for EntityRelay {
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
        inputs: Bytes,
        _region_rotation: i32,
    ) -> RelayFuture<'a, Bytes> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let asked: Value = serde_json::from_slice(&inputs).unwrap_or_default();
        let id: u64 = asked["id"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .unwrap_or_default();
        let answer = match method_id {
            "sc.track_by_id" => json!({"ok": true, "track": track(id)}),
            "sc.playlist_full" => json!({"ok": true, "playlist": playlist(id)}),
            other => panic!("an inline read never runs {other}"),
        };
        Box::pin(async move { Ok(Bytes::from(answer.to_string())) })
    }
}

fn track(id: u64) -> Value {
    json!({
        "id": id,
        "kind": "track",
        "urn": format!("soundcloud:tracks:{id}"),
        "title": "Opened from a link",
        "duration": 200000,
        "full_duration": 200000,
        "sharing": "public",
        "policy": "ALLOW",
        "user": {"id": id + 1, "kind": "user", "username": "Uploader"}
    })
}

fn playlist(id: u64) -> Value {
    json!({
        "id": id,
        "kind": "playlist",
        "urn": format!("soundcloud:playlists:{id}"),
        "title": "Opened mix",
        "track_count": 3,
        "sharing": "public",
        "user": {"id": id + 1, "kind": "user", "username": "Uploader"}
    })
}

fn unique_id() -> u64 {
    7_000_000_000 + u64::from(uuid::Uuid::now_v7().as_fields().1) * 1000
}

fn admission(per_user: u32) -> Arc<PublicAdmission> {
    let open = AdmissionLimitCfg {
        per_client: 100_000,
        global: 100_000,
    };
    PublicAdmission::for_live_search(
        redis(),
        AdmissionCfg {
            window: Duration::from_secs(60),
            timeout: Duration::from_millis(500),
            max_in_flight: 64,
            login: open,
            link_create: open,
            resolve: open,
            live_main: open,
            live_side: open,
            live_import: open,
            live_rescue: open,
            live_proxy: open,
            live_entity: AdmissionLimitCfg {
                per_client: per_user,
                global: 100_000,
            },
        },
    )
}

fn stash(pool: &PgPool, per_user: u32) -> anyhow::Result<(Arc<LiveStash>, Arc<EntityRelay>)> {
    let relay = Arc::new(EntityRelay {
        calls: AtomicUsize::new(0),
    });
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
    let misses = EntityMiss::new(read, admission(per_user), pool.clone());
    Ok((
        LiveStash::with_inline_reads(CacheService::new(redis()), misses),
        relay,
    ))
}

fn listener() -> String {
    format!("soundcloud:users:{}", unique_id())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_track_nobody_sighted_is_read_once_and_kept(pool: PgPool) -> anyhow::Result<()> {
    let (stash, relay) = stash(&pool, 10)?;
    let id = unique_id();

    assert!(
        stash
            .read_track(
                &indexing(&pool).await?,
                &format!("soundcloud:tracks:{id}"),
                &listener()
            )
            .await
    );
    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = $1")
        .bind(id.to_string())
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Opened from a link");
    assert_eq!(relay.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_playlist_nobody_sighted_keeps_its_metadata(pool: PgPool) -> anyhow::Result<()> {
    let (stash, _) = stash(&pool, 10)?;
    let id = unique_id();

    assert!(
        stash
            .read_playlist(&format!("soundcloud:playlists:{id}"), &listener())
            .await
    );
    let title: String = sqlx::query_scalar("SELECT title FROM playlists WHERE sc_playlist_id = $1")
        .bind(id.to_string())
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Opened mix");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_listener_past_the_budget_is_left_to_the_refresh_path(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (stash, relay) = stash(&pool, 1)?;
    let indexing = indexing(&pool).await?;
    let who = listener();

    assert!(
        stash
            .read_track(
                &indexing,
                &format!("soundcloud:tracks:{}", unique_id()),
                &who
            )
            .await
    );
    assert!(
        !stash
            .read_track(
                &indexing,
                &format!("soundcloud:tracks:{}", unique_id()),
                &who
            )
            .await
    );
    assert_eq!(relay.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_pause_row_stops_inline_reads(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sc_egress_health (channel, open_until, opened_by) \
         VALUES ('search_live_pause', now() + interval '1 hour', 'ops')",
    )
    .execute(&pool)
    .await?;
    let (stash, relay) = stash(&pool, 10)?;

    assert!(
        !stash
            .read_track(
                &indexing(&pool).await?,
                &format!("soundcloud:tracks:{}", unique_id()),
                &listener()
            )
            .await
    );
    assert_eq!(relay.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn without_the_flag_a_miss_never_reaches_soundcloud(pool: PgPool) -> anyhow::Result<()> {
    let stash = LiveStash::new(CacheService::new(redis()));

    assert!(
        !stash
            .read_track(
                &indexing(&pool).await?,
                &format!("soundcloud:tracks:{}", unique_id()),
                &listener()
            )
            .await
    );
    Ok(())
}
