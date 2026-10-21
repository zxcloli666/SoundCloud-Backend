use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::test_helpers::{
    Answer, call, caller, conversation, device, incarnation, member_in, messages, position,
    redis_db, redis_pool, revoke, settle, sync, user, with_postgres,
};
use crate::modules::chat::presence;
use crate::modules::chat::sync::bump;
use crate::modules::chat::sync::rows::{self, Fetch};
use crate::modules::chat::test_support::chat_state;

struct Session {
    user: String,
    device: Uuid,
    pos: u64,
    inc: Uuid,
}

impl Session {
    fn body(&self, hold_ms: u64, extra: Value) -> Value {
        let mut body = json!({
            "pos": self.pos,
            "inc": self.inc,
            "hold_ms": hold_ms,
            "max_bytes": 262_144,
        });
        if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
            body.extend(extra.clone());
        }
        body
    }

    fn follow(&mut self, answer: &Answer) {
        if let Some(pos) = answer.body["pos"].as_u64() {
            self.pos = pos;
        }
    }
}

async fn start(chat: &std::sync::Arc<crate::modules::chat::ChatState>, user: &str) -> Session {
    let device = device(&chat.pg, user, "active").await;
    let first = sync(chat, caller(user, device), json!({"max_bytes": 262_144})).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.body["reset"], true);
    Session {
        user: user.to_owned(),
        device,
        pos: first.body["pos"].as_u64().expect("a pos"),
        inc: first.body["inc"]
            .as_str()
            .and_then(|inc| inc.parse().ok())
            .expect("an incarnation"),
    }
}

fn entry(answer: &Answer, conversation: Uuid) -> &Value {
    answer.body["convs"]
        .as_array()
        .and_then(|convs| {
            convs
                .iter()
                .find(|entry| entry["id"] == conversation.to_string())
        })
        .unwrap_or_else(|| panic!("{conversation} is not in {}", answer.body))
}

fn seqs(entry: &Value) -> Vec<i64> {
    entry["messages"]
        .as_array()
        .map(|messages| messages.iter().filter_map(|m| m["seq"].as_i64()).collect())
        .unwrap_or_default()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_new_client_is_reset_then_a_change_wakes_its_parked_poll(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    let session = start(&chat, &bob).await;
    assert_eq!(session.inc, incarnation(&pool).await);

    let parked = {
        let chat = chat.clone();
        let caller = caller(&bob, session.device);
        let body = session.body(10_000, json!({"known": [[chat_id, 0]]}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 1).await;
    messages(&pool, chat_id, &alice, 2, 32).await;
    bump::conversation(&chat, chat_id, 2, &[alice.clone(), bob.clone()])
        .await
        .expect("a bump");

    let woke = tokio::time::timeout(Duration::from_secs(3), parked)
        .await
        .expect("the poll wakes")
        .expect("a poll");
    assert_eq!(woke.status, StatusCode::OK, "{}", woke.body);
    assert_eq!(woke.body["reset"], false);
    assert!(woke.body["pos"].as_u64() > Some(session.pos));
    let found = entry(&woke, chat_id);
    assert_eq!(seqs(found), vec![1, 2]);
    assert_eq!(found["last_seq"], 2);
    assert_eq!(found["more"], false);
    assert!(found["messages"][0]["body"].as_str().is_some());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn wakes_inside_the_window_coalesce_into_one_answer(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    let mut session = start(&chat, &bob).await;
    let known = json!({"known": [[chat_id, 0]]});

    let first = {
        let chat = chat.clone();
        let caller = caller(&bob, session.device);
        let body = session.body(10_000, known.clone());
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 1).await;
    let woken_at = Instant::now();
    messages(&pool, chat_id, &alice, 1, 16).await;
    bump::conversation(&chat, chat_id, 1, std::slice::from_ref(&bob))
        .await
        .expect("a bump");
    let first = first.await.expect("a poll");
    assert!(woken_at.elapsed() < Duration::from_millis(400));
    session.follow(&first);

    let second = {
        let chat = chat.clone();
        let caller = caller(&bob, session.device);
        let body = session.body(10_000, json!({"known": [[chat_id, 1]]}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 1).await;
    for seq in 2..=4 {
        messages(&pool, chat_id, &alice, 1, 16).await;
        bump::conversation(&chat, chat_id, seq, std::slice::from_ref(&bob))
            .await
            .expect("a bump");
    }
    let latest = position(&chat, &bob).await;
    let second = second.await.expect("a poll");
    assert!(woken_at.elapsed() >= Duration::from_millis(450));
    assert_eq!(second.body["pos"].as_u64(), Some(latest));
    assert_eq!(seqs(entry(&second, chat_id)), vec![2, 3, 4]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn typing_wakes_only_the_poll_focused_on_that_chat(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "group", &[&alice, &bob]).await;
    let mut focused = start(&chat, &bob).await;
    let elsewhere = Session {
        device: Uuid::now_v7(),
        user: bob.clone(),
        pos: focused.pos,
        inc: focused.inc,
    };

    let opened = sync(
        &chat,
        caller(&bob, focused.device),
        focused.body(10_000, json!({"focus": chat_id})),
    )
    .await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.body);
    assert_eq!(entry(&opened, chat_id)["typing"], json!([]));
    focused.follow(&opened);

    let focused_poll = {
        let chat = chat.clone();
        let caller = caller(&focused.user, focused.device);
        let body = focused.body(5_000, json!({"focus": chat_id}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    let unfocused_poll = {
        let chat = chat.clone();
        let caller = caller(&elsewhere.user, elsewhere.device);
        let body = elsewhere.body(1_500, json!({}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 2).await;

    let typed = call(
        &chat,
        caller(&alice, Uuid::now_v7()),
        "POST",
        &format!("/chat/conversations/{chat_id}/typing"),
        None,
    )
    .await;
    assert_eq!(typed.status, StatusCode::NO_CONTENT, "{}", typed.body);

    let woke = tokio::time::timeout(Duration::from_secs(2), focused_poll)
        .await
        .expect("the focused poll wakes")
        .expect("a poll");
    assert_eq!(woke.status, StatusCode::OK, "{}", woke.body);
    assert_eq!(entry(&woke, chat_id)["typing"], json!([alice]));
    assert_eq!(woke.body["pos"].as_u64(), Some(focused.pos));

    let unfocused = unfocused_poll.await.expect("a poll");
    assert_eq!(unfocused.status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn typing_in_a_chat_the_caller_is_not_in_is_refused(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob, mallory) = (user(), user(), user());
    let chat_id = conversation(&pool, "group", &[&alice, &bob]).await;
    let typed = call(
        &chat,
        caller(&mallory, Uuid::now_v7()),
        "POST",
        &format!("/chat/conversations/{chat_id}/typing"),
        None,
    )
    .await;
    assert_eq!(typed.status, StatusCode::FORBIDDEN);
    assert_eq!(typed.body, json!({"error": "chat_not_member"}));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_flushed_redis_resets_the_client(pool: PgPool) {
    let redis = redis_pool(&redis_db(9));
    let chat = chat_state(pool.clone(), redis.clone(), true);
    let bob = user();
    let mut session = start(&chat, &bob).await;

    let quiet = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(quiet.status, StatusCode::NO_CONTENT);

    let mut conn = redis.get().await.expect("redis");
    let _: () = redis::cmd("FLUSHDB")
        .query_async(&mut conn)
        .await
        .expect("a flush");
    let reset = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.body);
    assert_eq!(reset.body["reset"], true);
    session.follow(&reset);

    let settled = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(settled.status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn expired_messages_and_left_chats_reach_the_client(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "group", &[&alice, &bob]).await;
    let mut session = start(&chat, &bob).await;

    bump::messages_gone(&chat, chat_id, &[3, 4], std::slice::from_ref(&bob))
        .await
        .expect("a bump");
    let expired = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(expired.status, StatusCode::OK, "{}", expired.body);
    assert_eq!(
        expired.body["gone_msgs"],
        json!([[chat_id, 3], [chat_id, 4]])
    );
    session.follow(&expired);

    member_in(&pool, chat_id, &bob, "left").await;
    bump::conversation(&chat, chat_id, 0, std::slice::from_ref(&bob))
        .await
        .expect("a bump");
    let left = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(left.body["gone"], json!([chat_id]));
    assert_eq!(left.body["convs"], json!([]));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn missed_rows_come_from_postgres_and_stop_at_the_byte_cap(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let first = conversation(&pool, "group", &[&alice, &bob]).await;
    let second = conversation(&pool, "group", &[&alice, &bob]).await;
    messages(&pool, first, &alice, 20, 2_000).await;
    messages(&pool, second, &alice, 3, 10).await;
    let session = start(&chat, &bob).await;
    bump::conversation(&chat, first, 20, std::slice::from_ref(&bob))
        .await
        .expect("a bump");
    bump::conversation(&chat, second, 3, std::slice::from_ref(&bob))
        .await
        .expect("a bump");

    let mut body = session.body(0, json!({"known": [[first, 0], [second, 1]]}));
    body["max_bytes"] = json!(20_000);
    let answer = sync(&chat, caller(&bob, session.device), body).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let cut = entry(&answer, first);
    let delivered = seqs(cut);
    assert!(!delivered.is_empty() && delivered.len() < 20);
    assert_eq!(delivered, (1..=delivered.len() as i64).collect::<Vec<_>>());
    assert_eq!(cut["more"], true);
    assert!(answer.body.to_string().len() <= 20_000);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_dm_read_mark_reaches_the_peer(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    messages(&pool, chat_id, &alice, 2, 16).await;
    let sender = start(&chat, &alice).await;
    let reader = start(&chat, &bob).await;

    let marked = sync(
        &chat,
        caller(&bob, reader.device),
        reader.body(
            0,
            json!({"read": [[chat_id, 2]], "delivered": [[chat_id, 9]]}),
        ),
    )
    .await;
    assert!(marked.status.is_success(), "{}", marked.body);

    let seen = sync(
        &chat,
        caller(&alice, sender.device),
        sender.body(0, json!({})),
    )
    .await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.body);
    assert_eq!(
        entry(&seen, chat_id)["marks"],
        json!([{"user_id": bob, "read_seq": 2, "delivered_seq": 2}])
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_peer_without_read_receipts_shows_only_delivered(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    sqlx::query("INSERT INTO chat.users (user_id, read_receipts) VALUES ($1, false)")
        .bind(&bob)
        .execute(&pool)
        .await
        .expect("settings");
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    messages(&pool, chat_id, &alice, 3, 16).await;
    let sender = start(&chat, &alice).await;
    let reader = start(&chat, &bob).await;

    sync(
        &chat,
        caller(&bob, reader.device),
        reader.body(0, json!({"read": [[chat_id, 3]]})),
    )
    .await;
    let seen = sync(
        &chat,
        caller(&alice, sender.device),
        sender.body(0, json!({})),
    )
    .await;
    assert_eq!(
        entry(&seen, chat_id)["marks"],
        json!([{"user_id": bob, "read_seq": 0, "delivered_seq": 3}])
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn marks_for_a_chat_the_caller_is_not_in_are_dropped(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob, mallory) = (user(), user(), user());
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    messages(&pool, chat_id, &alice, 2, 16).await;
    let sender = start(&chat, &alice).await;
    let intruder = start(&chat, &mallory).await;

    sync(
        &chat,
        caller(&mallory, intruder.device),
        intruder.body(0, json!({"read": [[chat_id, 2]]})),
    )
    .await;
    let quiet = sync(
        &chat,
        caller(&alice, sender.device),
        sender.body(0, json!({})),
    )
    .await;
    assert_eq!(quiet.status, StatusCode::NO_CONTENT, "{}", quiet.body);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn devices_and_requests_come_when_their_markers_move(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let mut session = start(&chat, &bob).await;
    assert_eq!(session_devices(&chat, &session).await, 1);

    let laptop = device(&pool, &bob, "active").await;
    chat.devices.forget(&bob, &[]);
    bump::devices(&chat, &bob).await.expect("a bump");
    let listed = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    let listed_ids: Vec<(Value, Value)> = listed.body["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(|row| (row["id"].clone(), row["current"].clone()))
        .collect();
    assert_eq!(
        listed_ids,
        vec![
            (json!(session.device), json!(true)),
            (json!(laptop), json!(false))
        ]
    );
    session.follow(&listed);

    let invite = conversation(&pool, "direct", &[&alice]).await;
    member_in(&pool, invite, &bob, "invited").await;
    bump::requests(&chat, &bob).await.expect("a bump");
    let requested = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(requested.body["requests_count"], 1);
    assert_eq!(requested.body["devices"], json!([]));
    session.follow(&requested);

    bump::conversation(&chat, invite, 0, std::slice::from_ref(&bob))
        .await
        .expect("a bump");
    let cached = sync(
        &chat,
        caller(&bob, session.device),
        session.body(0, json!({})),
    )
    .await;
    assert_eq!(cached.body["requests_count"], 1);
    assert_eq!(entry(&cached, invite)["last_seq"], 0);
}

async fn session_devices(
    chat: &std::sync::Arc<crate::modules::chat::ChatState>,
    session: &Session,
) -> usize {
    let answer = sync(
        chat,
        caller(&session.user, session.device),
        json!({"max_bytes": 1}),
    )
    .await;
    answer.body["devices"].as_array().map_or(0, Vec::len)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn presence_lists_only_sharing_contacts_seen_recently(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob, carol, dave, erin) = (user(), user(), user(), user(), user());
    for (person, shares) in [
        (&alice, true),
        (&bob, true),
        (&carol, true),
        (&dave, true),
        (&erin, false),
    ] {
        sqlx::query("INSERT INTO chat.users (user_id, presence_share) VALUES ($1, $2)")
            .bind(person)
            .bind(shares)
            .execute(&pool)
            .await
            .expect("settings");
    }
    conversation(&pool, "direct", &[&alice, &bob]).await;
    conversation(&pool, "direct", &[&alice, &erin]).await;
    for (from, to) in [(&alice, &dave), (&dave, &alice), (&alice, &carol)] {
        sqlx::query(
            "INSERT INTO user_followings (user_id, target_user_urn) VALUES ($1, 'soundcloud:users:' || $2)",
        )
        .bind(from)
        .bind(to)
        .execute(&pool)
        .await
        .expect("a follow");
    }
    for person in [&bob, &carol, &dave, &erin] {
        presence::mark_seen(&chat, person).await.expect("seen");
    }

    let mut expected = vec![bob.clone(), dave.clone()];
    expected.sort();
    assert_eq!(
        presence::online_contacts(&chat, &alice)
            .await
            .expect("contacts"),
        expected
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_read_on_one_device_reaches_the_other_devices_of_that_user(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "group", &[&alice, &bob]).await;
    messages(&pool, chat_id, &alice, 3, 16).await;
    let phone = start(&chat, &bob).await;
    let desktop = start(&chat, &bob).await;

    let parked = {
        let chat = chat.clone();
        let caller = caller(&bob, desktop.device);
        let body = desktop.body(5_000, json!({"known": [[chat_id, 3]]}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 1).await;
    sync(
        &chat,
        caller(&bob, phone.device),
        phone.body(0, json!({"read": [[chat_id, 3]]})),
    )
    .await;

    let woke = tokio::time::timeout(Duration::from_secs(2), parked)
        .await
        .expect("the other device wakes")
        .expect("a poll");
    assert_eq!(woke.status, StatusCode::OK, "{}", woke.body);
    assert_eq!(
        entry(&woke, chat_id)["marks"],
        json!([{"user_id": bob, "read_seq": 3, "delivered_seq": 3}])
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_device_revoked_while_parked_gets_no_more_changes(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    let session = start(&chat, &bob).await;

    let parked = {
        let chat = chat.clone();
        let caller = caller(&bob, session.device);
        let body = session.body(5_000, json!({"known": [[chat_id, 0]]}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 1).await;
    revoke(&pool, session.device).await;
    chat.devices.forget(&bob, &[session.device]);
    messages(&pool, chat_id, &alice, 1, 16).await;
    bump::conversation(&chat, chat_id, 1, std::slice::from_ref(&bob))
        .await
        .expect("a bump");

    let ended = tokio::time::timeout(Duration::from_secs(2), parked)
        .await
        .expect("the poll ends")
        .expect("a poll");
    assert_eq!(ended.body, json!({"error": "chat_device_revoked"}));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_focus_on_a_chat_the_caller_is_not_in_is_checked_once(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob, mallory) = (user(), user(), user());
    let chat_id = conversation(&pool, "group", &[&alice, &bob]).await;
    let mut session = start(&chat, &mallory).await;

    let refused = sync(
        &chat,
        caller(&mallory, session.device),
        session.body(300, json!({"focus": chat_id})),
    )
    .await;
    assert_eq!(refused.status, StatusCode::OK, "{}", refused.body);
    assert_eq!(refused.body["gone"], json!([chat_id]));
    session.follow(&refused);

    let started = Instant::now();
    let held = sync(
        &chat,
        caller(&mallory, session.device),
        session.body(300, json!({"focus": chat_id})),
    )
    .await;
    assert_eq!(held.status, StatusCode::NO_CONTENT, "{}", held.body);
    assert!(started.elapsed() >= Duration::from_millis(250));

    let parked = {
        let chat = chat.clone();
        let caller = caller(&mallory, session.device);
        let body = session.body(500, json!({"focus": chat_id}));
        tokio::spawn(async move { sync(&chat, caller, body).await })
    };
    settle(&chat, 1).await;
    call(
        &chat,
        caller(&alice, Uuid::now_v7()),
        "POST",
        &format!("/chat/conversations/{chat_id}/typing"),
        None,
    )
    .await;
    let quiet = parked.await.expect("a poll");
    assert_eq!(quiet.status, StatusCode::NO_CONTENT, "{}", quiet.body);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn typing_between_blocked_dm_peers_is_dropped(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let chat_id = conversation(&pool, "direct", &[&alice, &bob]).await;
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ($1, $2)")
        .bind(&alice)
        .bind(&bob)
        .execute(&pool)
        .await
        .expect("a block");

    let typed = call(
        &chat,
        caller(&bob, Uuid::now_v7()),
        "POST",
        &format!("/chat/conversations/{chat_id}/typing"),
        None,
    )
    .await;
    assert_eq!(typed.status, StatusCode::NO_CONTENT, "{}", typed.body);
    assert!(
        chat.hub
            .typing
            .candidates(chat_id, tokio::time::Instant::now())
            .is_empty()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn presence_is_hidden_between_blocked_contacts(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob, carol) = (user(), user(), user());
    for person in [&alice, &bob, &carol] {
        sqlx::query("INSERT INTO chat.users (user_id, presence_share) VALUES ($1, true)")
            .bind(person)
            .execute(&pool)
            .await
            .expect("settings");
    }
    conversation(&pool, "direct", &[&alice, &bob]).await;
    conversation(&pool, "direct", &[&alice, &carol]).await;
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ($1, $2)")
        .bind(&carol)
        .bind(&alice)
        .execute(&pool)
        .await
        .expect("a block");
    for person in [&bob, &carol] {
        presence::mark_seen(&chat, person).await.expect("seen");
    }

    assert_eq!(
        presence::online_contacts(&chat, &alice)
            .await
            .expect("contacts"),
        vec![bob.clone()]
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_postgres_fallback_spends_one_budget_across_chats(pool: PgPool) {
    let chat = with_postgres(&pool);
    let alice = user();
    let first = conversation(&pool, "group", &[&alice]).await;
    let second = conversation(&pool, "group", &[&alice]).await;
    messages(&pool, first, &alice, 10, 1_000).await;
    messages(&pool, second, &alice, 10, 1_000).await;
    let wanted = [
        Fetch {
            conversation: first,
            after: 0,
        },
        Fetch {
            conversation: second,
            after: 0,
        },
    ];

    let found = rows::fetch(&chat, &wanted, 8_000).await.expect("rows");
    let first_seqs: Vec<i64> = found[&first].iter().map(|row| row.seq).collect();
    assert_eq!(first_seqs, (1..=6).collect::<Vec<_>>());
    assert!(!found.contains_key(&second));
}
