use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::caller::{CallerDevice, ChatCaller};
use crate::modules::chat::routes::inner;
use crate::modules::chat::test_support::{caller, offline_chat_state};

const CONV: &str = "0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f";

fn with_device() -> ChatCaller {
    ChatCaller {
        device: Some(CallerDevice {
            id: Uuid::from_u128(0xd0),
            created_at: chrono::Utc::now(),
            history_unlocked: true,
        }),
        ..caller()
    }
}

async fn answer(
    chat: Arc<ChatState>,
    who: ChatCaller,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = inner().with_state(chat);
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let mut request = request
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    request.extensions_mut().insert(who);
    let response = router.oneshot(request).await.expect("an answer");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn invalid() -> (StatusCode, Value) {
    (StatusCode::BAD_REQUEST, json!({"error": "chat_invalid"}))
}

fn send_body(class: &str) -> Value {
    json!({
        "client_id": "0192e5c5-0000-7000-8000-000000000001",
        "class": class,
        "epoch": 0,
        "body": "AQID"
    })
}

#[tokio::test]
async fn a_timer_outside_the_three_values_is_refused_before_any_query() {
    let uri = format!("/chat/conversations/{CONV}/ttl");
    assert_eq!(
        answer(
            offline_chat_state(true),
            caller(),
            "PUT",
            &uri,
            Some(json!({"ttl_s": 60}))
        )
        .await,
        invalid()
    );
}

#[tokio::test]
async fn a_room_chat_is_never_created_through_this_route() {
    assert_eq!(
        answer(
            offline_chat_state(true),
            caller(),
            "POST",
            "/chat/conversations",
            Some(json!({"kind": "room", "members": []}))
        )
        .await,
        invalid()
    );
}

#[tokio::test]
async fn a_dm_with_yourself_is_refused() {
    assert_eq!(
        answer(
            offline_chat_state(true),
            caller(),
            "POST",
            "/chat/conversations",
            Some(json!({"kind": "direct", "members": ["101"]}))
        )
        .await,
        invalid()
    );
}

#[tokio::test]
async fn a_send_needs_a_device() {
    let uri = format!("/chat/conversations/{CONV}/messages");
    assert_eq!(
        answer(
            offline_chat_state(true),
            caller(),
            "POST",
            &uri,
            Some(send_body("post"))
        )
        .await,
        invalid()
    );
}

#[tokio::test]
async fn a_system_row_from_a_client_is_refused() {
    let uri = format!("/chat/conversations/{CONV}/messages");
    assert_eq!(
        answer(
            offline_chat_state(true),
            with_device(),
            "POST",
            &uri,
            Some(send_body("system"))
        )
        .await,
        invalid()
    );
}

#[tokio::test]
async fn the_send_kill_switch_answers_first() {
    let Ok(mut chat) = Arc::try_unwrap(offline_chat_state(true)) else {
        panic!("the state has one owner");
    };
    chat.config.send_paused = true;
    let paused = Arc::new(chat);
    let uri = format!("/chat/conversations/{CONV}/messages");
    assert_eq!(
        answer(paused, with_device(), "POST", &uri, Some(send_body("post"))).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "chat_send_paused"})
        )
    );
}

#[tokio::test]
async fn delete_for_me_is_not_a_server_message_delete() {
    let uri = format!("/chat/conversations/{CONV}/messages/5?for=me");
    assert_eq!(
        answer(offline_chat_state(true), caller(), "DELETE", &uri, None).await,
        invalid()
    );
}

#[tokio::test]
async fn a_bad_cursor_and_a_missing_scope_are_invalid() {
    assert_eq!(
        answer(
            offline_chat_state(true),
            caller(),
            "GET",
            "/chat/conversations?cursor=nope",
            None
        )
        .await,
        invalid()
    );
    let uri = format!("/chat/conversations/{CONV}");
    assert_eq!(
        answer(offline_chat_state(true), caller(), "DELETE", &uri, None).await,
        invalid()
    );
}

mod wired {
    use redis::AsyncCommands;
    use sqlx::PgPool;

    use super::*;
    use chat_wire::msg::Class;

    use crate::modules::chat::ChatError;
    use crate::modules::chat::conversations::fixtures::{DEVICE, dm, group, outgoing, post};
    use crate::modules::chat::members::access;
    use crate::modules::chat::messages::{HistoryPage, HistoryQuery, gate, page, rules, store};
    use crate::modules::chat::model::MemberState;
    use crate::modules::chat::rt_keys;
    use crate::modules::chat::test_support::chat_state;
    use crate::modules::rooms::model::{Member, Profile, Room, new_code};
    use crate::modules::rooms::store::RoomStore;

    fn redis_url() -> String {
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
    }

    fn as_user(user: &str) -> ChatCaller {
        ChatCaller {
            user_id: user.to_owned(),
            ..with_device()
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires a local Redis"]
    async fn a_dm_timer_reaches_the_row_the_view_and_the_bump(pool: PgPool) -> anyhow::Result<()> {
        let chat = chat_state(pool.clone(), crate::redis::connect_url(&redis_url())?, true);
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        let ttl = format!("/chat/conversations/{id}/ttl");
        let (status, body) = answer(
            chat.clone(),
            as_user("202"),
            "PUT",
            &ttl,
            Some(json!({"ttl_s": 86_400})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({"ttl_s": 86_400, "seq": 1}));
        let cached = chat.hot.after(id, 0).expect("the system row is cached");
        assert_eq!(cached[0].class, "system");
        let mut conn = chat.redis.get().await?;
        let seq: i64 = conn.get(rt_keys::conversation_seq(id)).await?;
        assert_eq!(seq, 1);
        let (status, view) = answer(
            chat.clone(),
            as_user("101"),
            "GET",
            &format!("/chat/conversations/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["ttl_s"], 86_400);
        assert_eq!(view["peer"], "202");
        assert_eq!(view["me"]["state"], "active");
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires a local Redis"]
    async fn member_routes_answer_their_shapes(pool: PgPool) -> anyhow::Result<()> {
        let chat = chat_state(pool.clone(), crate::redis::connect_url(&redis_url())?, true);
        let id = group(
            &pool,
            "101",
            &[("202", MemberState::Active), ("303", MemberState::Invited)],
        )
        .await;
        let base = format!("/chat/conversations/{id}");
        let (status, members) = answer(
            chat.clone(),
            as_user("202"),
            "GET",
            &format!("{base}/members"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{members}");
        assert_eq!(members.as_array().map(Vec::len), Some(3));
        assert_eq!(members[0]["role"], "owner");
        let (status, _) = answer(
            chat.clone(),
            as_user("303"),
            "POST",
            &format!("{base}/accept"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, roles) = answer(
            chat.clone(),
            as_user("101"),
            "PUT",
            &format!("{base}/members/202"),
            Some(json!({"role": "admin"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{roles}");
        let (status, _) = answer(
            chat.clone(),
            as_user("202"),
            "DELETE",
            &format!("{base}/members/303?ban=true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, mine) = answer(
            chat.clone(),
            as_user("202"),
            "PUT",
            &format!("{base}/me"),
            Some(
                json!({"muted_until": 4_102_444_800_000_i64, "notify": "mentions", "pinned": true}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["me"]["notify"], "mentions");
        assert_eq!(mine["me"]["muted_until"], 4_102_444_800_000_i64);
        assert!(mine["me"]["pinned_at"].is_i64());
        let (status, unmuted) = answer(
            chat.clone(),
            as_user("202"),
            "PUT",
            &format!("{base}/me"),
            Some(json!({"muted_until": null})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unmuted["me"]["muted_until"], Value::Null);
        assert_eq!(unmuted["me"]["notify"], "mentions");
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires a local Redis"]
    async fn deletes_answer_no_content_and_leave_a_trace(pool: PgPool) -> anyhow::Result<()> {
        let chat = chat_state(pool.clone(), crate::redis::connect_url(&redis_url())?, true);
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        let sent = post(&pool, id, "101").await;
        let base = format!("/chat/conversations/{id}");
        let (status, _) = answer(
            chat.clone(),
            as_user("101"),
            "DELETE",
            &format!("{base}/messages/{}?for=everyone", sent.seq),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let cached = chat.hot.after(id, 0).expect("a cached tail");
        assert_eq!(
            cached.iter().map(|row| row.seq).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(cached[0].body, None);
        let (status, _) = answer(
            chat.clone(),
            as_user("202"),
            "DELETE",
            &format!("{base}?for=me"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, view) = answer(chat.clone(), as_user("202"), "GET", &base, None).await;
        assert_eq!(view["me"]["hidden_before_seq"], 2);
        let (status, _) = answer(
            chat.clone(),
            as_user("202"),
            "DELETE",
            &format!("{base}?for=everyone"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(chat.hot.after(id, 0).is_none());
        let (status, gone) = answer(chat, as_user("101"), "GET", &base, None).await;
        assert_eq!(
            (status, gone),
            (StatusCode::FORBIDDEN, json!({"error": "chat_not_member"}))
        );
        Ok(())
    }

    fn seated(user: &str, from_seq: i64) -> Member {
        let profile = Profile {
            name: format!("User {user}"),
            avatar_url: None,
        };
        Member {
            chat_from_seq: from_seq,
            ..Member::new(user, &profile, 10)
        }
    }

    async fn room_chat(chat: &ChatState, host: &str, guest: &str, from_seq: i64) -> Uuid {
        let id = Uuid::now_v7();
        let code = new_code();
        sqlx::query(
            "INSERT INTO chat.conversations (id, kind, room_ref, created_by) VALUES ($1, 'room', $2, $3)",
        )
        .bind(id)
        .bind(&code)
        .bind(host)
        .execute(&chat.pg)
        .await
        .expect("a room chat row");
        let mut room = Room::new(code.clone(), seated(host, 0), false, 10);
        room.members.push(seated(guest, from_seq));
        room.chat_id = Some(id);
        room.slow_mode_s = Some(60);
        let store = RoomStore::new(chat.redis.clone());
        assert!(store.insert(&room).await.expect("a stored room"));
        store.index_chat(id, &code).await.expect("a chat index");
        id
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires a local Redis"]
    async fn room_chats_take_their_seats_from_the_room(pool: PgPool) -> anyhow::Result<()> {
        let chat = chat_state(pool.clone(), crate::redis::connect_url(&redis_url())?, true);
        let id = room_chat(&chat, "101", "202", 2).await;
        assert_eq!(
            access::load_in(&chat, id, "303").await.err(),
            Some(ChatError::NotMember)
        );
        let host = access::load_in(&chat, id, "101").await.expect("the host");
        let guest = access::load_in(&chat, id, "202").await.expect("a guest");
        assert!(host.moderates() && !guest.moderates());
        assert_eq!((guest.joined_seq, guest.history_floor()), (2, 2));
        assert_eq!(guest.recipients, vec!["101".to_owned(), "202".to_owned()]);
        for _ in 0..2 {
            let out = outgoing(Class::Post, 0, None);
            assert!(
                gate::room(&chat, &host, &out)
                    .await
                    .expect("open")
                    .is_none()
            );
            store::store(&pool, &host, DEVICE, &out, None)
                .await
                .expect("a host post");
        }
        let out = outgoing(Class::Post, 0, None);
        assert!(
            gate::room(&chat, &guest, &out)
                .await
                .expect("open")
                .is_none()
        );
        let first = store::store(&pool, &guest, DEVICE, &out, None)
            .await
            .expect("a guest post");
        assert_eq!(first.seq, 3);
        let again = gate::room(&chat, &guest, &out).await.expect("a resend");
        assert_eq!(again.map(|row| row.seq), Some(3));
        assert!(matches!(
            gate::room(&chat, &guest, &outgoing(Class::Post, 0, None)).await,
            Err(ChatError::SlowMode { .. })
        ));
        assert_eq!(
            rules::may_post(&guest, Class::Pin),
            Err(ChatError::NotMember)
        );
        let newest = HistoryQuery {
            after: None,
            before: None,
            limit: None,
        };
        let seqs = |page: HistoryPage| page.messages.iter().map(|row| row.seq).collect::<Vec<_>>();
        assert_eq!(
            seqs(
                page::read(&pool, &guest, &newest, 1 << 20)
                    .await
                    .expect("a page")
            ),
            vec![3]
        );
        assert_eq!(
            seqs(
                page::read(&pool, &host, &newest, 1 << 20)
                    .await
                    .expect("a page")
            ),
            vec![1, 2, 3]
        );
        let messages = format!("/chat/conversations/{id}/messages");
        let (status, _) = answer(
            chat.clone(),
            as_user("202"),
            "DELETE",
            &format!("{messages}/1?for=everyone"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = answer(
            chat.clone(),
            as_user("101"),
            "DELETE",
            &format!("{messages}/3?for=everyone"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        let cached = chat.hot.after(id, 2).expect("a cached tail");
        assert_eq!(cached[0].body, None);
        let mut conn = chat.redis.get().await?;
        let told: Option<f64> = conn.zscore(rt_keys::deltas("202"), id.to_string()).await?;
        assert!(told.is_some());
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires a local Redis"]
    async fn a_dm_send_goes_through_every_gate_once(pool: PgPool) -> anyhow::Result<()> {
        let chat = chat_state(pool.clone(), crate::redis::connect_url(&redis_url())?, true);
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        let messages = format!("/chat/conversations/{id}/messages");
        let body = json!({
            "client_id": Uuid::now_v7(),
            "class": "post",
            "epoch": 0,
            "body": "AQID"
        });
        let (status, first) = answer(
            chat.clone(),
            as_user("101"),
            "POST",
            &messages,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let (status, again) =
            answer(chat.clone(), as_user("101"), "POST", &messages, Some(body)).await;
        assert_eq!(
            (status, again["seq"].clone()),
            (StatusCode::OK, first["seq"].clone())
        );
        let (status, page) = answer(chat.clone(), as_user("202"), "GET", &messages, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["messages"][0]["seq"], first["seq"]);
        sqlx::query(
            "INSERT INTO chat.users (user_id, restricted_until, restriction_scope)
             VALUES ('101', now() + interval '1 day', '{all}')",
        )
        .execute(&pool)
        .await?;
        let (status, muted) = answer(
            chat,
            as_user("101"),
            "POST",
            &messages,
            Some(json!({"client_id": Uuid::now_v7(), "class": "post", "epoch": 0, "body": "AQID"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{muted}");
        assert_eq!(muted["error"], "chat_muted");
        Ok(())
    }
}
