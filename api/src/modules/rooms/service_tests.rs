use std::sync::Arc;
use std::time::Duration;

use redis::AsyncCommands;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::modules::chat::rooms::bridge;
use crate::modules::chat::rooms::lookup::recipients;
use crate::modules::chat::rt_keys;
use crate::modules::chat::test_support::chat_state;
use crate::modules::rooms::model::{MAX_MEMBERS, SettingsPatch};

struct Bench {
    chat: Arc<ChatState>,
    pg: PgPool,
    redis: Pool,
}

impl Bench {
    fn new(pg: PgPool) -> anyhow::Result<Self> {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let redis = crate::redis::connect_url(&url)?;
        let chat = chat_state(pg.clone(), redis.clone(), true);
        chat.rooms.install_chat(&chat);
        Ok(Self { chat, pg, redis })
    }

    fn rooms(&self) -> &RoomsService {
        &self.chat.rooms
    }

    async fn open_chat(&self, code: &str, user: &str) -> anyhow::Result<Uuid> {
        let opened = self.rooms().claim_chat(code, user).await?;
        bridge::ensure_conversation(&self.chat, &opened)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        Ok(opened.conversation_id)
    }

    async fn system_rows(&self, conversation: Uuid) -> anyhow::Result<Vec<Value>> {
        let bodies: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT body FROM chat.messages WHERE conversation_id = $1 AND class = 'system' ORDER BY seq",
        )
        .bind(conversation)
        .fetch_all(&self.pg)
        .await?;
        Ok(bodies
            .iter()
            .map(|body| serde_json::from_slice(body).unwrap_or(Value::Null))
            .collect())
    }

    async fn told_gone(&self, user: &str, conversation: Uuid) -> anyhow::Result<bool> {
        let mut conn = self.redis.get().await?;
        let score: Option<f64> = conn
            .zscore(rt_keys::deltas(user), conversation.to_string())
            .await?;
        Ok(score.is_some())
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        let mut conn = self.redis.get().await?;
        Ok(conn.exists(key).await?)
    }
}

fn someone() -> String {
    format!("8{:011}", Uuid::now_v7().as_u128() % 100_000_000_000)
}

fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        avatar_url: None,
    }
}

fn playing() -> PlaybackUpdate {
    serde_json::from_value(json!({
        "status": "playing",
        "trackUrn": "42",
        "track": {"title": "Night Drive", "user": {"username": "Owls"}},
        "positionMs": 1_000
    }))
    .expect("a playback update")
}

fn types(rows: &[Value]) -> Vec<&str> {
    rows.iter().filter_map(|row| row["type"].as_str()).collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_poll_from_outside_the_room_leaves_no_trace(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, stranger) = (someone(), someone());
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), false, None)
        .await?;

    let refused = bench
        .rooms()
        .wait(&room.code, &stranger, Some(1), Duration::from_secs(5), true)
        .await
        .unwrap_err();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let seen = bench.rooms().store.seen_since(&room.code, 0).await?;
    assert_eq!(seen, std::slice::from_ref(&host));

    let ghost = new_code();
    let refused = bench
        .rooms()
        .wait(&ghost, &stranger, Some(1), Duration::from_secs(5), true)
        .await
        .unwrap_err();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert!(!bench.exists(&format!("rooms:{ghost}:seen")).await?);
    assert!(!bench.exists(&format!("rooms:{ghost}")).await?);
    let mut conn = bench.redis.get().await?;
    let alive: Option<f64> = conn.zscore(rt_keys::ROOMS_ALIVE, &ghost).await?;
    assert_eq!(alive, None);
    bench.rooms().leave(&room.code, &host).await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_member_poll_keeps_the_room_and_its_chat_alive(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), false, None)
        .await?;
    let conversation = bench.open_chat(&room.code, &host).await?;
    sqlx::query("UPDATE chat.conversations SET room_alive_until = now() WHERE id = $1")
        .bind(conversation)
        .execute(&bench.pg)
        .await?;
    let mut conn = bench.redis.get().await?;
    let _: bool = conn.expire(format!("rooms:{}", room.code), 100).await?;
    bench.rooms().alive.invalidate(&room.code);

    bench
        .rooms()
        .wait(&room.code, &host, None, Duration::ZERO, false)
        .await?;
    let ttl: i64 = conn.ttl(format!("rooms:{}", room.code)).await?;
    assert!(ttl > 3_600, "{ttl}");
    let alive: Option<f64> = conn.zscore(rt_keys::ROOMS_ALIVE, &room.code).await?;
    assert!(alive.is_some());
    let indexed: Option<String> = conn.get(format!("rooms:chat:{conversation}")).await?;
    assert_eq!(indexed.as_deref(), Some(room.code.as_str()));
    let left: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM room_alive_until - now())::float8 FROM chat.conversations WHERE id = $1",
    )
    .bind(conversation)
    .fetch_one(&bench.pg)
    .await?;
    assert!(left > 500.0, "{left}");
    bench.rooms().leave(&room.code, &host).await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn seats_of_absent_guests_go_and_their_chat_is_told(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), false, None)
        .await?;
    let code = room.code.clone();
    let guests: Vec<String> = (0..MAX_MEMBERS - 1).map(|_| someone()).collect();
    for guest in &guests {
        bench
            .rooms()
            .join(&code, guest, &profile("Guest"), &bench.pg)
            .await?;
    }
    let conversation = bench.open_chat(&code, &host).await?;
    bench.rooms().set_playback(&code, &host, playing()).await?;
    bench
        .rooms()
        .apply(&code, Cause::Quiet, |room| {
            for member in &mut room.members {
                member.joined_at -= 10 * 60_000;
            }
            Ok(())
        })
        .await?;
    let stayed = &guests[0];
    for guest in &guests[1..] {
        bench.rooms().store.forget(&code, guest).await?;
    }
    let before = bridge::last_seq(&bench.chat, conversation).await;
    assert_eq!(before, 1, "the track row");
    let late = someone();
    bench
        .rooms()
        .join(&code, &late, &profile("Late"), &bench.pg)
        .await?;

    let room = bench.rooms().room(&code).await?;
    let left: Vec<&str> = room.members.iter().map(|m| m.user_id.as_str()).collect();
    assert_eq!(left, [host.as_str(), stayed.as_str(), late.as_str()]);
    assert_eq!(room.member(&late).map(|m| m.chat_from_seq), Some(before));
    for guest in &guests[1..] {
        assert!(bench.told_gone(guest, conversation).await?, "{guest}");
    }
    assert!(bench.told_gone(&late, conversation).await?);
    let rekey: bool = sqlx::query_scalar("SELECT rekey FROM chat.conversations WHERE id = $1")
        .bind(conversation)
        .fetch_one(&bench.pg)
        .await?;
    assert!(rekey);
    let rows = bench.system_rows(conversation).await?;
    assert_eq!(types(&rows).iter().filter(|t| **t == "leave").count(), 8);
    assert_eq!(
        rows.last().map(|row| row["type"].clone()),
        Some(json!("join"))
    );
    assert_eq!(
        rows.last().map(|row| row["user_id"].clone()),
        Some(json!(late))
    );
    let mut heard = recipients(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    heard.sort();
    let mut expected = vec![host.clone(), stayed.clone(), late.clone()];
    expected.sort();
    assert_eq!(heard, expected);
    bench.rooms().end(&code).await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_room_that_goes_private_keeps_its_chat_and_its_bans(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest, troll) = (someone(), someone(), someone());
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), true, Some("Open"))
        .await?;
    let code = room.code.clone();
    for user in [&guest, &troll] {
        bench
            .rooms()
            .join_public(&host, user, &profile("Guest"), &bench.pg)
            .await?;
    }
    let conversation = bench.open_chat(&code, &guest).await?;
    bench.rooms().kick(&code, &host, &troll, true).await?;
    assert!(bench.told_gone(&troll, conversation).await?);
    let listed = bench.rooms().public_rooms().await?;
    let card = listed.iter().find(|card| card.host_id == host);
    assert_eq!(
        card.map(|card| card.banned.clone()),
        Some(vec![troll.clone()])
    );
    assert_eq!(
        card.and_then(|card| card.title.clone()).as_deref(),
        Some("Open")
    );
    let hidden = bench
        .rooms()
        .join_public(&host, &troll, &profile("Troll"), &bench.pg)
        .await
        .unwrap_err();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let moved = bench.rooms().set_public(&code, &host, false).await?;
    assert_ne!(moved.code, code);
    assert!(bench.rooms().find(&code).await?.is_none());
    let found = bench
        .rooms()
        .room_by_chat(conversation)
        .await?
        .expect("the chat follows the room");
    assert_eq!(found.code, moved.code);
    assert_eq!(found.banned, std::slice::from_ref(&troll));
    let members: Vec<&str> = found.members.iter().map(|m| m.user_id.as_str()).collect();
    assert_eq!(members, [host.as_str(), guest.as_str()]);
    let mut conn = bench.redis.get().await?;
    let stale: Option<f64> = conn.zscore(rt_keys::ROOMS_ALIVE, &code).await?;
    assert_eq!(stale, None);

    let banned = bench
        .rooms()
        .join(&moved.code, &troll, &profile("Troll"), &bench.pg)
        .await
        .unwrap_err();
    assert_eq!(banned.status(), StatusCode::FORBIDDEN);
    bench.rooms().end(&moved.code).await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_host_hands_the_room_over_with_its_code_chat_and_music(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, first, second) = (someone(), someone(), someone());
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), false, None)
        .await?;
    let code = room.code.clone();
    bench
        .rooms()
        .join(&code, &first, &profile("First"), &bench.pg)
        .await?;
    tokio::time::sleep(Duration::from_millis(5)).await;
    bench
        .rooms()
        .join(&code, &second, &profile("Second"), &bench.pg)
        .await?;
    bench.rooms().set_playback(&code, &host, playing()).await?;
    let conversation = bench.open_chat(&code, &first).await?;
    bench
        .rooms()
        .set_moderator(&code, &host, &first, true)
        .await?;
    let before = bench.rooms().room(&code).await?;

    bench.rooms().leave(&code, &host).await?;

    let after = bench.rooms().room(&code).await?;
    assert_eq!(after.host_id, first);
    assert_eq!(after.code, code);
    assert_eq!(after.chat_id, Some(conversation));
    assert_eq!(after.playback, before.playback);
    assert!(after.moderators.is_empty());
    assert_eq!(
        bench.rooms().directory.hosted_by(&first).await?,
        Some(code.clone())
    );
    assert_eq!(bench.rooms().directory.hosted_by(&host).await?, None);
    assert!(bench.told_gone(&host, conversation).await?);
    let rows = bench.system_rows(conversation).await?;
    let tail: Vec<&str> = types(&rows).into_iter().rev().take(2).collect();
    assert_eq!(tail, ["host", "leave"]);
    let handed = rows.last().cloned().unwrap_or_default();
    assert_eq!(handed["user_id"], json!(first));
    assert_eq!(handed["name"], "First");
    let view = bench.rooms().view(&code, &second).await?;
    assert_eq!(view.host_id, first);

    bench.rooms().store.forget(&code, &second).await?;
    bench.rooms().leave(&code, &first).await?;
    assert!(
        bench.rooms().find(&code).await?.is_none(),
        "nobody online ends the room"
    );
    assert!(bench.told_gone(&second, conversation).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_last_one_out_ends_the_room_and_shreds_its_chat(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), true, None)
        .await?;
    let code = room.code.clone();
    bench
        .rooms()
        .join(&code, &guest, &profile("Guest"), &bench.pg)
        .await?;
    let conversation = bench.open_chat(&code, &host).await?;
    sqlx::query(
        "INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version) VALUES ($1, 0, '\\x00', 1)",
    )
    .bind(conversation)
    .execute(&bench.pg)
    .await?;

    bench.rooms().leave(&code, &guest).await?;
    assert!(bench.rooms().find(&code).await?.is_some());
    bench.rooms().leave(&code, &host).await?;

    assert!(!bench.exists(&format!("rooms:{code}")).await?);
    assert!(!bench.exists(&format!("rooms:{code}:seen")).await?);
    assert!(!bench.exists(&format!("rooms:chat:{conversation}")).await?);
    assert_eq!(bench.rooms().directory.hosted_by(&host).await?, None);
    let mut conn = bench.redis.get().await?;
    let alive: Option<f64> = conn.zscore(rt_keys::ROOMS_ALIVE, &code).await?;
    assert_eq!(alive, None);
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chat.conversations WHERE id = $1")
            .bind(conversation)
            .fetch_one(&bench.pg)
            .await?;
    assert!(deleted);
    let keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.conversation_keys WHERE conversation_id = $1",
    )
    .bind(conversation)
    .fetch_one(&bench.pg)
    .await?;
    assert_eq!(keys, 0);
    assert!(bench.told_gone(&host, conversation).await?);
    assert!(bench.told_gone(&guest, conversation).await?);
    assert!(
        !bench
            .rooms()
            .public_rooms()
            .await?
            .iter()
            .any(|card| card.host_id == host)
    );
    assert!(!bench.rooms().end(&code).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn nobody_the_host_blocked_gets_in(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, blocked, muted_artist) = (someone(), someone(), someone());
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), true, None)
        .await?;
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ($1, $2)")
        .bind(&host)
        .bind(&blocked)
        .execute(&bench.pg)
        .await?;
    sqlx::query(
        "INSERT INTO user_blocked_artists (sc_user_id, kind, target_id, name, sc_user_ids)
         VALUES ($1, 'user', $2, 'Them', ARRAY[$2])",
    )
    .bind(&host)
    .bind(&muted_artist)
    .execute(&bench.pg)
    .await?;
    for user in [&blocked, &muted_artist] {
        let refused = bench
            .rooms()
            .join(&room.code, user, &profile("Them"), &bench.pg)
            .await
            .unwrap_err();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{user}");
        let refused = bench
            .rooms()
            .join_public(&host, user, &profile("Them"), &bench.pg)
            .await
            .unwrap_err();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{user}");
    }
    bench.rooms().end(&room.code).await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_preview_touches_nothing_and_is_cached(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, stranger) = (someone(), someone());
    let room = bench
        .rooms()
        .create(&host, &profile("Host"), false, Some("Late show"))
        .await?;
    let code = room.code.clone();
    bench.rooms().set_playback(&code, &host, playing()).await?;

    let preview = bench.rooms().preview(&code).await?.expect("a live room");
    assert_eq!(preview.title.as_deref(), Some("Late show"));
    assert_eq!(preview.host.id, host);
    assert_eq!(preview.listeners, 1);
    assert_eq!(
        preview.track.as_ref().map(|track| track.urn.as_str()),
        Some("soundcloud:tracks:42")
    );
    let seen = bench.rooms().store.seen_since(&code, 0).await?;
    assert_eq!(seen, std::slice::from_ref(&host));
    assert!(
        bench
            .rooms()
            .find(&code)
            .await?
            .is_some_and(|room| !room.is_member(&stranger))
    );

    bench
        .rooms()
        .set_playback(
            &code,
            &host,
            serde_json::from_value(json!({"status": "idle"}))?,
        )
        .await?;
    let cached = bench.rooms().preview(&code).await?.expect("still cached");
    assert!(cached.track.is_some(), "served from the 10 s cache");

    let renamed = SettingsPatch {
        title: Some("Encore".to_owned()),
        ..SettingsPatch::default()
    };
    bench.rooms().configure(&code, &host, &renamed).await?;
    let fresh = bench.rooms().preview(&code).await?.expect("a live room");
    assert_eq!(fresh.title.as_deref(), Some("Encore"));
    assert!(fresh.track.is_none());

    assert!(bench.rooms().preview(&new_code()).await?.is_none());
    bench.rooms().end(&code).await?;
    assert!(bench.rooms().preview(&code).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn room_json_from_the_old_api_is_served_and_upgraded(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    let code = new_code();
    let legacy = json!({
        "code": code,
        "hostId": host,
        "version": 3,
        "createdAt": 1,
        "members": [
            {"userId": host, "name": "Host", "avatarUrl": null, "joinedAt": 1, "readyUrn": null}
        ],
        "playback": {"status": "idle", "track": null, "trackUrn": null, "positionMs": 0, "at": 0, "rate": 1.0},
        "public": false
    });
    let mut conn = bench.redis.get().await?;
    let _: () = conn
        .hset_multiple(
            format!("rooms:{code}"),
            &[("v", "3".to_owned()), ("body", legacy.to_string())],
        )
        .await?;
    let _: bool = conn.expire(format!("rooms:{code}"), 600).await?;

    let view = bench
        .rooms()
        .join(&code, &guest, &profile("Guest"), &bench.pg)
        .await?;
    assert_eq!(view.members.len(), 2);
    assert_eq!(view.version, 4);
    assert!(view.chat_enabled);
    let stored: String = conn.hget(format!("rooms:{code}"), "body").await?;
    let stored: Value = serde_json::from_str(&stored)?;
    assert_eq!(stored["members"][1]["chatFromSeq"], 0);
    assert_eq!(stored["chatEnabled"], true);
    bench.rooms().end(&code).await?;
    Ok(())
}
