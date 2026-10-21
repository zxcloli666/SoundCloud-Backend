use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use redis::AsyncCommands;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::bridge;
use super::diff::{departed, rows};
use super::lookup::{gate_send, room_of_chat};
use super::member_from_seq;
use super::refusal;
use super::system::SuggestedTrack;
use crate::error::AppError;
use crate::modules::chat::caller::ChatCaller;
use crate::modules::chat::devices::test_support::{Enrolled, call, caller, enroll};
use crate::modules::chat::model::Tier;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatState, routes, rt_keys};
use crate::modules::rooms::model::{Member, PlaybackUpdate, Profile, Room, banned, not_member};
use crate::modules::rooms::service::Cause;

fn member(id: &str, joined_at: i64) -> Member {
    Member::new(
        id,
        &Profile {
            name: format!("User {id}"),
            avatar_url: None,
        },
        joined_at,
    )
}

fn room() -> Room {
    let mut room = Room::new("ABC234".to_owned(), member("1", 10), false, 10);
    room.join(member("2", 20)).unwrap();
    room.join(member("3", 30)).unwrap();
    room.chat_id = Some(Uuid::from_u128(7));
    room
}

fn shown(before: &Room, after: &Room, cause: Cause<'_>) -> Vec<(String, Value)> {
    rows(before, after, cause, 1_000)
        .into_iter()
        .map(|authored| {
            (
                authored.sender,
                serde_json::to_value(&authored.row).unwrap(),
            )
        })
        .collect()
}

fn playing(urn: &str) -> PlaybackUpdate {
    serde_json::from_value(json!({
        "status": "playing",
        "trackUrn": urn,
        "track": {"title": " Night Drive ", "user": {"username": "Owls"}},
        "positionMs": 0
    }))
    .unwrap()
}

#[test]
fn a_guest_who_leaves_and_one_who_is_shown_out_read_differently() {
    let before = room();
    let mut after = before.clone();
    after.remove("2");
    assert_eq!(departed(&before, &after), ["2"]);
    assert_eq!(
        shown(&before, &after, Cause::Member),
        [(
            "2".to_owned(),
            json!({"type": "leave", "user_id": "2", "name": "User 2"})
        )]
    );
    let mut kicked = before.clone();
    kicked.kick("1", "2").unwrap();
    assert_eq!(
        shown(
            &before,
            &kicked,
            Cause::Moderation {
                by: "1",
                ban: false
            }
        ),
        [(
            "1".to_owned(),
            json!({"type": "kick", "user_id": "2", "name": "User 2", "by": "1", "ban": false})
        )]
    );
    let mut banned = before.clone();
    banned.ban("1", "2").unwrap();
    let rows = shown(&before, &banned, Cause::Moderation { by: "1", ban: true });
    assert_eq!(rows[0].1["ban"], true);
}

#[test]
fn every_visible_change_of_a_room_leaves_one_row() {
    let before = room();
    let mut after = before.clone();
    after.join(member("4", 40)).unwrap();
    assert_eq!(
        shown(&before, &after, Cause::Member)[0].1,
        json!({"type": "join", "user_id": "4", "name": "User 4"})
    );

    let mut handed = before.clone();
    handed.hand_off("1", &["3".to_owned()]);
    let rows = shown(&before, &handed, Cause::Member);
    let kinds: Vec<&Value> = rows.iter().map(|(_, row)| &row["type"]).collect();
    assert_eq!(kinds, [&json!("leave"), &json!("host")]);
    assert_eq!(
        rows[1],
        (
            "3".to_owned(),
            json!({"type": "host", "user_id": "3", "name": "User 3"})
        )
    );

    let mut moderated = before.clone();
    moderated.set_moderator("1", "2", true).unwrap();
    moderated.mute("1", "3", 60, 1_000).unwrap();
    moderated.title = Some("Late".to_owned());
    let by_host = Cause::Moderation {
        by: "1",
        ban: false,
    };
    let rows: Vec<Value> = shown(&before, &moderated, by_host)
        .into_iter()
        .map(|(_, row)| row)
        .collect();
    assert_eq!(
        rows,
        [
            json!({"type": "moderator", "user_id": "2", "name": "User 2", "by": "1", "on": true}),
            json!({"type": "mute", "user_id": "3", "name": "User 3", "by": "1", "until_ms": 61_000}),
            json!({"type": "title", "by": "1", "title": "Late"}),
        ]
    );
    let mut unmuted = moderated.clone();
    unmuted.mute("1", "3", 0, 1_000).unwrap();
    assert_eq!(
        shown(&moderated, &unmuted, by_host)[0].1["until_ms"],
        Value::Null
    );

    let mut tuned = before.clone();
    tuned.set_playback("1", playing("42"), 100).unwrap();
    assert_eq!(
        shown(&before, &tuned, Cause::Member),
        [(
            "1".to_owned(),
            json!({"type": "track", "urn": "soundcloud:tracks:42", "title": "Night Drive",
                   "artist": "Owls", "artwork_url": null})
        )]
    );
    let mut seeked = tuned.clone();
    seeked.set_playback("1", playing("42"), 900).unwrap();
    assert!(shown(&tuned, &seeked, Cause::Member).is_empty());
}

#[test]
fn a_change_nobody_sees_leaves_no_row() {
    let before = room();
    let mut ready = before.clone();
    ready.mark_ready("2", "42").unwrap();
    assert!(shown(&before, &ready, Cause::Member).is_empty());
    let mut chat_opened = before.clone();
    chat_opened.chat_id = Some(Uuid::from_u128(9));
    assert!(shown(&before, &chat_opened, Cause::Quiet).is_empty());
}

#[test]
fn a_suggested_track_is_a_small_clean_snapshot() {
    let track = SuggestedTrack::from_snapshot(&json!({
        "urn": "soundcloud:tracks:42",
        "title": "  Song\u{0007} ",
        "artist": "Owls",
        "artworkUrl": "https://i1.sndcdn.com/artworks-1.jpg",
        "durationMs": 180_000
    }))
    .unwrap();
    assert_eq!(
        track,
        SuggestedTrack {
            urn: "soundcloud:tracks:42".to_owned(),
            title: "Song".to_owned(),
            artist: Some("Owls".to_owned()),
            artwork_url: Some("https://i1.sndcdn.com/artworks-1.jpg".to_owned()),
            duration_ms: Some(180_000),
        }
    );
    let api_shape = SuggestedTrack::from_snapshot(&json!({
        "urn": "42",
        "title": "Song",
        "user": {"username": "Owls"},
        "artwork_url": "https://evil.example/a.jpg",
        "duration": 1_000
    }))
    .unwrap();
    assert_eq!(api_shape.artist.as_deref(), Some("Owls"));
    assert_eq!(api_shape.artwork_url, None);
    assert_eq!(api_shape.urn, "soundcloud:tracks:42");
    for broken in [
        json!({"urn": "soundcloud:users:42", "title": "Song"}),
        json!({"urn": "soundcloud:tracks:42"}),
        json!({"urn": "soundcloud:tracks:42", "title": "x".repeat(4_000)}),
        json!("soundcloud:tracks:42"),
    ] {
        assert_eq!(SuggestedTrack::from_snapshot(&broken), None, "{broken}");
    }
}

#[test]
fn room_refusals_become_chat_errors() {
    assert_eq!(refusal(not_member()), ChatError::NotMember);
    assert_eq!(refusal(banned()), ChatError::Unavailable);
    assert_eq!(refusal(AppError::not_found("x")), ChatError::NotFound);
    assert_eq!(refusal(AppError::bad_request("x")), ChatError::Invalid);
    assert_eq!(
        refusal(AppError::conflict("x")),
        ChatError::Busy { retry_ms: 500 }
    );
    assert_eq!(refusal(AppError::internal("x")), ChatError::Internal);
    let mut room = room();
    for id in ["2", "3"] {
        room.set_moderator("1", id, true).unwrap();
    }
    room.join(member("5", 50)).unwrap();
    room.set_moderator("1", "5", true).unwrap();
    room.join(member("6", 60)).unwrap();
    let full = room.set_moderator("1", "6", true).unwrap_err();
    assert_eq!(refusal(full), ChatError::Invalid);
}

fn someone() -> String {
    format!("7{:011}", Uuid::now_v7().as_u128() % 100_000_000_000)
}

fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        avatar_url: None,
    }
}

async fn ask(
    router: &axum::Router,
    user: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))?;
    request.extensions_mut().insert(ChatCaller {
        user_id: user.to_owned(),
        session_id: Uuid::now_v7(),
        device: None,
        tier: Tier::Direct,
        max_bytes: 262_144,
    });
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

fn bench(pg: PgPool) -> anyhow::Result<(Arc<ChatState>, axum::Router)> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let chat = chat_state(pg, crate::redis::connect_url(&url)?, true);
    chat.rooms.install_chat(&chat);
    let router = routes::inner().with_state(chat.clone());
    Ok((chat, router))
}

async fn system_types(chat: &ChatState, conversation: Uuid) -> anyhow::Result<Vec<Value>> {
    let bodies: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT body FROM chat.messages WHERE conversation_id = $1 AND class = 'system' ORDER BY seq",
    )
    .bind(conversation)
    .fetch_all(&chat.pg)
    .await?;
    Ok(bodies
        .iter()
        .filter_map(|body| serde_json::from_slice(body).ok())
        .collect())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_room_is_moderated_through_the_sealed_routes(pg: PgPool) -> anyhow::Result<()> {
    let (chat, router) = bench(pg)?;
    let (host, keeper, guest, troll, stranger) =
        (someone(), someone(), someone(), someone(), someone());
    let code = chat
        .rooms
        .create(&host, &profile("Host"), true, None)
        .await?
        .code;
    for user in [&keeper, &guest, &troll] {
        chat.rooms
            .join(&code, user, &profile("Guest"), &chat.pg)
            .await?;
    }
    let base = format!("/chat/rooms/{code}");

    let (status, body) = ask(&router, &stranger, "GET", &format!("{base}/chat"), None).await?;
    assert_eq!(
        (status, body),
        (StatusCode::FORBIDDEN, json!({"error": "chat_not_member"}))
    );
    let (status, opened) = ask(&router, &guest, "GET", &format!("{base}/chat"), None).await?;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(opened["from_seq"], 0);
    let conversation: Uuid = serde_json::from_value(opened["conversation_id"].clone())?;
    assert_eq!(
        room_of_chat(&chat, conversation)
            .await
            .map(|room| room.map(|room| room.code)),
        Ok(Some(code.clone()))
    );
    assert_eq!(
        member_from_seq(&chat, conversation, &guest).await,
        Ok(Some(0))
    );
    assert_eq!(
        member_from_seq(&chat, conversation, &stranger).await,
        Ok(None)
    );
    let (_, again) = ask(&router, &host, "GET", &format!("{base}/chat"), None).await?;
    assert_eq!(again["conversation_id"], opened["conversation_id"]);
    let (kind, room_ref, created_by): (String, String, String) =
        sqlx::query_as("SELECT kind, room_ref, created_by FROM chat.conversations WHERE id = $1")
            .bind(conversation)
            .fetch_one(&chat.pg)
            .await?;
    assert_eq!((kind.as_str(), room_ref.as_str()), ("room", code.as_str()));
    assert_eq!(created_by, host);

    let promote = format!("{base}/moderators/{keeper}");
    let on = Some(json!({"on": true}));
    let (status, _) = ask(&router, &guest, "PUT", &promote, on.clone()).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = ask(&router, &host, "PUT", &promote, on).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let mute = Some(json!({"seconds": 60}));
    let (status, _) = ask(
        &router,
        &keeper,
        "PUT",
        &format!("{base}/members/{guest}/mute"),
        mute,
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(matches!(
        gate_send(&chat, conversation, &guest, "post").await,
        Err(ChatError::Muted { .. })
    ));
    assert!(gate_send(&chat, conversation, &troll, "post").await.is_ok());
    assert!(matches!(
        gate_send(&chat, conversation, &troll, "post").await,
        Err(ChatError::SlowMode { .. })
    ));
    assert!(
        gate_send(&chat, conversation, &troll, "react")
            .await
            .is_ok()
    );
    for _ in 0..2 {
        assert!(
            gate_send(&chat, conversation, &keeper, "post")
                .await
                .is_ok()
        );
    }

    let ban = format!("{base}/members/{troll}?ban=true");
    let (status, _) = ask(&router, &keeper, "DELETE", &ban, None).await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "only the host bans");
    let (status, _) = ask(&router, &host, "DELETE", &ban, None).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        gate_send(&chat, conversation, &troll, "post").await,
        Err(ChatError::NotMember)
    );
    assert_eq!(member_from_seq(&chat, conversation, &troll).await, Ok(None));
    let mut conn = chat.redis.get().await?;
    let told: Option<f64> = conn
        .zscore(rt_keys::deltas(&troll), conversation.to_string())
        .await?;
    assert!(told.is_some(), "the banned guest learns the chat is gone");

    let settings = format!("{base}/settings");
    let patch = Some(json!({"title": "After hours", "slow_mode_s": 0}));
    let (status, _) = ask(&router, &keeper, "PUT", &settings, patch.clone()).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, view) = ask(&router, &host, "PUT", &settings, patch).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["title"], "After hours");
    assert_eq!(view["banned"], json!([troll]));
    assert_eq!(view["slowModeS"], 0);

    let track = Some(json!({"track": {"urn": "soundcloud:tracks:42", "title": "Song"}}));
    let suggest = format!("{base}/suggest");
    let (status, body) = ask(&router, &guest, "POST", &suggest, track.clone()).await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "chat_muted");
    let (status, _) = ask(&router, &keeper, "POST", &suggest, track.clone()).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = ask(&router, &keeper, "POST", &suggest, track).await?;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "chat_slow_mode");
    let (status, _) = ask(
        &router,
        &keeper,
        "POST",
        &suggest,
        Some(json!({"track": {"urn": "soundcloud:tracks:42"}})),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let rows = system_types(&chat, conversation).await?;
    let kinds: Vec<&str> = rows.iter().filter_map(|row| row["type"].as_str()).collect();
    assert_eq!(kinds, ["moderator", "mute", "kick", "title", "suggest"]);
    assert_eq!(rows[2]["ban"], true);
    assert_eq!(rows[4]["track"]["urn"], "soundcloud:tracks:42");
    assert_eq!(rows[4]["user_id"], json!(keeper));

    let (status, _) = ask(&router, &host, "GET", "/chat/rooms/BAD-01/chat", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    chat.rooms.end(&code).await?;
    let (status, body) = ask(&router, &host, "GET", &format!("{base}/chat"), None).await?;
    assert_eq!(
        (status, body),
        (StatusCode::NOT_FOUND, json!({"error": "chat_not_found"}))
    );
    Ok(())
}

fn room_epochs(answer: &Value) -> Vec<i64> {
    answer
        .as_array()
        .map(|keys| {
            keys.iter()
                .filter_map(|key| key["epoch"].as_i64())
                .collect()
        })
        .unwrap_or_default()
}

async fn room_keys(
    chat: &Arc<ChatState>,
    user: &str,
    device: &Enrolled,
    conversation: Uuid,
) -> (StatusCode, Value) {
    call(
        chat,
        caller(user, device.session, Some(&device.row)),
        "GET",
        &format!("/chat/conversations/{conversation}/keys"),
        None,
    )
    .await
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn room_keys_follow_the_seats_of_the_room(pg: PgPool) -> anyhow::Result<()> {
    let (chat, _) = bench(pg.clone())?;
    let (host, guest, stranger, late) = (someone(), someone(), someone(), someone());
    let code = chat
        .rooms
        .create(&host, &profile("Host"), false, None)
        .await?
        .code;
    chat.rooms
        .join(&code, &guest, &profile("Guest"), &chat.pg)
        .await?;
    let opened = chat.rooms.claim_chat(&code, &host).await?;
    bridge::ensure_conversation(&chat, &opened)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let conversation = opened.conversation_id;
    let host_device = enroll(&chat, &pg, &host).await;
    let guest_device = enroll(&chat, &pg, &guest).await;
    let stranger_device = enroll(&chat, &pg, &stranger).await;

    let (status, answer) = room_keys(&chat, &host, &host_device, conversation).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let first = room_epochs(&answer);
    assert_eq!(first.len(), 1, "{answer}");
    let (status, answer) = room_keys(&chat, &guest, &guest_device, conversation).await;
    assert_eq!(
        (status, room_epochs(&answer)),
        (StatusCode::OK, first.clone())
    );
    let (status, answer) = room_keys(&chat, &stranger, &stranger_device, conversation).await;
    assert_eq!(
        (status, answer),
        (StatusCode::FORBIDDEN, json!({"error": "chat_not_member"}))
    );

    chat.rooms.kick(&code, &host, &guest, false).await?;
    let (status, answer) = room_keys(&chat, &guest, &guest_device, conversation).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    let (_, answer) = room_keys(&chat, &host, &host_device, conversation).await;
    let rotated = room_epochs(&answer);
    assert_eq!(rotated.first(), first.first(), "{answer}");
    let newest = rotated.last().copied().unwrap_or_default();
    assert!(newest > first[0], "a departure rotates the key: {answer}");

    chat.rooms
        .join(&code, &late, &profile("Late"), &chat.pg)
        .await?;
    let late_device = enroll(&chat, &pg, &late).await;
    let (status, answer) = room_keys(&chat, &late, &late_device, conversation).await;
    assert_eq!(
        (status, room_epochs(&answer)),
        (StatusCode::OK, vec![newest])
    );
    chat.rooms.end(&code).await?;
    Ok(())
}
