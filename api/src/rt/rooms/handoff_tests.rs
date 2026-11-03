use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::http::StatusCode;
use redact::Secret;
use redis::AsyncCommands;
use serde_json::json;
use sqlx::PgPool;
use url::Url;

use super::{Proxy, hand_off};
use crate::modules::chat::test_support::chat_state;
use crate::modules::rooms::model::Profile;
use crate::router::router_search_tests::app_with_admin;
use crate::rt::test_support::{self, call, session_for};

const TOKEN: &str = "handoff-test-token";
const MOVE_DBS: (u8, u8) = (12, 13);
const PROXY_DBS: (u8, u8) = (14, 15);

fn url_of(var: &str, db: u8) -> String {
    let base = std::env::var(var)
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let mut url = Url::parse(&base).expect("a redis url");
    url.set_path(&format!("/{db}"));
    url.to_string()
}

fn music_url(db: u8) -> String {
    url_of("REDIS_URL", db)
}

fn rt_url(db: u8) -> String {
    url_of("REDIS_RT_URL", db)
}

async fn flushed(url: &str) -> deadpool_redis::Pool {
    let pool = crate::redis::connect_url(url).expect("a redis pool");
    let mut conn = pool.get().await.expect("a redis connection");
    let _: () = redis::cmd("FLUSHDB")
        .query_async(&mut conn)
        .await
        .expect("a flushed db");
    pool
}

async fn room_keys(pool: &deadpool_redis::Pool) -> BTreeMap<String, i64> {
    let mut conn = pool.get().await.expect("a redis connection");
    let keys: Vec<String> = conn.keys("rooms:*").await.expect("keys");
    let mut ttls = BTreeMap::new();
    for key in keys {
        let pttl: i64 = redis::cmd("PTTL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .expect("a pttl");
        ttls.insert(key, pttl);
    }
    ttls
}

async fn serve_api(pool: &PgPool, redis_url: &str) -> anyhow::Result<Url> {
    let app = app_with_admin(pool, redis_url, TOKEN).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("an api");
    });
    Ok(Url::parse(&format!("http://{address}"))?)
}

fn rooms_of(pool: deadpool_redis::Pool) -> std::sync::Arc<crate::modules::rooms::RoomsService> {
    crate::modules::rooms::RoomsService::new(pool, crate::modules::rooms::RoomHub::new(None))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_handoff_moves_every_room_with_its_ttl_and_a_second_call_finds_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let music = flushed(&music_url(MOVE_DBS.0)).await;
    let rt = flushed(&rt_url(MOVE_DBS.1)).await;
    let api = serve_api(&pool, &music_url(MOVE_DBS.0)).await?;
    let api_rooms = rooms_of(music.clone());
    let rt_rooms = rooms_of(rt.clone());
    let token = Secret::new(TOKEN.to_owned());

    let profile: Profile = serde_json::from_value(json!({"name": "Host"}))?;
    let mut codes = Vec::new();
    for host in 0..250 {
        let room = api_rooms
            .create(&format!("{}", 7_000 + host), &profile, host % 2 == 0, None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        codes.push(room.code);
    }
    {
        let mut conn = music.get().await?;
        let _: () = conn.set("rooms:custom-forever", "1").await?;
        let _: () = conn.set_ex("rooms:custom-short", "2", 90).await?;
    }
    let before = room_keys(&music).await;
    assert!(before.len() > 500, "{} keys", before.len());
    assert_eq!(before["rooms:custom-forever"], -1);

    let moved = hand_off(&api, &token, &rt_rooms).await?;

    assert_eq!(moved, before.len());
    assert!(room_keys(&music).await.is_empty());
    let after = room_keys(&rt).await;
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert_eq!(after["rooms:custom-forever"], -1);
    for (key, was) in &before {
        if *was > 0 {
            let now = after[key];
            assert!(
                now > 0 && now <= *was && *was - now < 10_000,
                "{key}: {was} -> {now}"
            );
        }
    }
    let first = codes[0].clone();
    let room = rt_rooms
        .room(&first)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(room.host_id, "7000");

    assert_eq!(hand_off(&api, &token, &rt_rooms).await?, 0);

    rt_rooms
        .leave(&first, "7000")
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(hand_off(&api, &token, &rt_rooms).await?, 0);
    assert!(
        rt_rooms
            .find(&first)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .is_none()
    );
    assert!(room_keys(&music).await.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_wrong_admin_token_is_refused_and_takes_nothing(pool: PgPool) -> anyhow::Result<()> {
    let music = flushed(&music_url(MOVE_DBS.0)).await;
    let api = serve_api(&pool, &music_url(MOVE_DBS.0)).await?;
    let mut conn = music.get().await?;
    let _: () = conn.set("rooms:keep", "1").await?;
    let http = wreq::Client::builder().build()?;
    let url = format!("{api}admin/rooms/handoff?cursor=0&count=200");

    let refused = super::import::ask(&http, &url, &Secret::new("not-it".to_owned())).await;

    assert!(refused.is_err());
    let present: bool = conn.exists("rooms:keep").await?;
    assert!(present);
    let page = super::import::ask(&http, &url, &Secret::new(TOKEN.to_owned())).await?;
    assert_eq!(page.keys.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn rooms_live_on_the_api_while_proxied_and_move_to_redis_rt_at_the_cut(
    pool: PgPool,
) -> anyhow::Result<()> {
    let music = flushed(&music_url(PROXY_DBS.0)).await;
    let rt = flushed(&rt_url(PROXY_DBS.1)).await;
    let api = serve_api(&pool, &music_url(PROXY_DBS.0)).await?;
    let proxied: Router = test_support::app(
        chat_state(pool.clone(), rt.clone(), true),
        &rt_url(PROXY_DBS.1),
        Some(Proxy::new(&api)),
    );
    let host = session_for(&pool, "9101").await?;
    let guest = session_for(&pool, "9202").await?;

    let (status, room) = call(
        &proxied,
        host,
        "POST",
        "/rooms",
        Some(json!({"name": "Host"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED, "{room}");
    let code = room["code"].as_str().unwrap_or_default().to_owned();
    let (status, room) = call(
        &proxied,
        guest,
        "POST",
        &format!("/rooms/{code}/members"),
        Some(json!({"name": "Guest"})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["members"].as_array().map(Vec::len), Some(2));
    assert!(room_keys(&rt).await.is_empty());
    assert!(
        room_keys(&music)
            .await
            .contains_key(&format!("rooms:{code}"))
    );
    let (status, _) = call(
        &proxied,
        uuid::Uuid::now_v7(),
        "GET",
        &format!("/rooms/{code}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let local = test_support::app(
        chat_state(pool.clone(), rt.clone(), true),
        &rt_url(PROXY_DBS.1),
        None,
    );
    let moved = hand_off(&api, &Secret::new(TOKEN.to_owned()), &rooms_of(rt.clone())).await?;
    assert!(moved >= 2);

    assert!(room_keys(&music).await.is_empty());
    let (status, room) = call(&local, guest, "GET", &format!("/rooms/{code}"), None).await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["hostId"], "9101");
    assert_eq!(room["members"].as_array().map(Vec::len), Some(2));
    assert_eq!(room["code"], code);
    Ok(())
}
