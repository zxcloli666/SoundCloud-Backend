use axum::http::StatusCode;
use chat_wire::bind::make_bind;
use chat_wire::prost::Message;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::test_support::{
    DeviceKeys, age, call, caller, conversation, desktop, enroll, fresh_device_budget, fresh_user,
    rekey_pending, session, state,
};
use super::{lookup, touch};
use crate::modules::chat::sync::changes::Change;
use crate::modules::chat::{ChatState, rt_keys};

async fn status_of(pool: &PgPool, device: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM chat.devices WHERE id = $1")
        .bind(device)
        .fetch_one(pool)
        .await
        .expect("a device")
}

async fn session_exists(pool: &PgPool, session: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1)")
        .bind(session)
        .fetch_one(pool)
        .await
        .expect("a probe")
}

fn error(answer: &Value) -> &str {
    answer["error"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_session_without_a_bind_must_log_in_again(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let bare = session(&pool, &user, None).await;
    let (status, answer) = call(
        &chat,
        caller(&user, bare, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::UNAUTHORIZED, "chat_relogin_required")
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_forged_bind_signature_must_log_in_again(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let keys = DeviceKeys::new();
    let mut bind = make_bind(&keys.id.to_string(), &keys.auth_seed, &keys.box_pub, 1);
    bind.box_pub = DeviceKeys::new().box_pub.to_vec();
    let forged = session(&pool, &user, Some(bind.encode_to_vec())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, forged, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::UNAUTHORIZED, "chat_relogin_required")
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_mismatched_bind_is_refused(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let first = enroll(&chat, &pool, &user).await;
    let other = DeviceKeys::new();
    let bound_elsewhere = session(&pool, &user, Some(other.bind())).await;

    let (status, answer) = call(
        &chat,
        caller(&user, bound_elsewhere, Some(&first.row)),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::CONFLICT, "chat_session_bound_other"),
        "a device cannot register on a session bound to another device"
    );

    let mut swapped = DeviceKeys::new();
    swapped.id = first.keys.id;
    let reused_id = session(&pool, &user, Some(swapped.bind())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, reused_id, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::CONFLICT, "chat_session_bound_other"),
        "a known device id with other keys is somebody else"
    );

    let stranger = DeviceKeys::new();
    let foreign = session(&pool, "7999", Some(stranger.bind())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, foreign, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::CONFLICT, "chat_session_bound_other"),
        "a session of another SoundCloud user never registers for this one"
    );

    assert_eq!(status_of(&pool, first.keys.id).await, "active");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_relogin_with_the_same_keys_moves_the_device(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let first = enroll(&chat, &pool, &user).await;
    let chat_id = conversation(&pool, "direct", &[user.as_str(), "7002"]).await;
    sqlx::query("UPDATE chat.conversations SET rekey = false")
        .execute(&pool)
        .await
        .expect("a reset");

    let relogin = session(&pool, &user, Some(first.keys.bind())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, relogin, Some(&first.row)),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["id"], json!(first.keys.id));
    assert_eq!(answer["current"], json!(true));

    let moved = lookup(&chat, first.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    assert_eq!(moved.session_id, relogin);
    assert_eq!(moved.created_at, first.row.created_at);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.devices")
        .fetch_one(&pool)
        .await
        .expect("a count");
    assert_eq!(count, 1);
    assert!(
        !rekey_pending(&pool, chat_id).await,
        "the same device coming back is not a new reader"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_new_device_rekeys_every_live_conversation_of_its_user(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let direct = conversation(&pool, "direct", &[user.as_str(), "7002"]).await;
    let group = conversation(&pool, "group", &[user.as_str(), "7002", "7003"]).await;
    let left = conversation(&pool, "group", &[user.as_str(), "7003"]).await;
    let foreign = conversation(&pool, "direct", &["7002", "7003"]).await;
    sqlx::query(
        "UPDATE chat.members SET state = 'left' WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(left)
    .bind(&user)
    .execute(&pool)
    .await
    .expect("a departure");

    enroll(&chat, &pool, &user).await;

    assert!(rekey_pending(&pool, direct).await);
    assert!(rekey_pending(&pool, group).await);
    assert!(!rekey_pending(&pool, left).await);
    assert!(!rekey_pending(&pool, foreign).await);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_session_keeps_one_device(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let first = enroll(&chat, &pool, &user).await;
    let second = DeviceKeys::new();
    sqlx::query("UPDATE sessions SET chat_bind = $2 WHERE id = $1")
        .bind(first.session)
        .bind(second.bind())
        .execute(&pool)
        .await
        .expect("a rebind");

    let (status, answer) = call(
        &chat,
        caller(&user, first.session, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(status_of(&pool, first.keys.id).await, "revoked");
    assert_eq!(status_of(&pool, second.id).await, "active");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_user_keeps_at_most_ten_devices(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    for _ in 0..10 {
        enroll(&chat, &pool, &user).await;
    }
    let extra = DeviceKeys::new();
    let extra_session = session(&pool, &user, Some(extra.bind())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, extra_session, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::TOO_MANY_REQUESTS, "chat_request_limit")
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_malformed_device_description_is_refused(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let keys = DeviceKeys::new();
    let bound = session(&pool, &user, Some(keys.bind())).await;
    for body in [
        json!({"platform": "", "name": "Mac", "app_version": "8.7.0"}),
        json!({"platform": "mac os", "name": "Mac", "app_version": "8.7.0"}),
        json!({"platform": "macos", "name": "  ", "app_version": "8.7.0"}),
        json!({"platform": "macos", "name": "Mac\n", "app_version": "8.7.0", "caps": ["a b"]}),
        json!({"platform": "macos", "name": "x".repeat(65), "app_version": "8.7.0"}),
    ] {
        let (status, answer) = call(
            &chat,
            caller(&user, bound, None),
            "POST",
            "/chat/devices",
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            (status, error(&answer)),
            (StatusCode::BAD_REQUEST, "chat_invalid"),
            "{body}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_young_device_cannot_remove_an_older_one(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let older = enroll(&chat, &pool, &user).await;
    let older_row = age(&chat, &pool, &older.row, 2).await;
    let younger = enroll(&chat, &pool, &user).await;

    for path in [
        format!("/chat/devices/{}", older.keys.id),
        format!("/chat/devices/{}/not-me", older.keys.id),
    ] {
        let method = if path.ends_with("not-me") {
            "POST"
        } else {
            "DELETE"
        };
        let (status, answer) = call(
            &chat,
            caller(&user, younger.session, Some(&younger.row)),
            method,
            &path,
            None,
        )
        .await;
        assert_eq!(
            (status, error(&answer)),
            (StatusCode::FORBIDDEN, "chat_too_young"),
            "{method} {path}"
        );
    }
    assert_eq!(status_of(&pool, older.keys.id).await, "active");
    assert!(session_exists(&pool, older.session).await);

    let (status, _) = call(
        &chat,
        caller(&user, older.session, Some(&older_row)),
        "DELETE",
        &format!("/chat/devices/{}", younger.keys.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(status_of(&pool, younger.keys.id).await, "revoked");
    assert!(
        !session_exists(&pool, younger.session).await,
        "removing a device ends its session"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_device_seasoned_for_72_hours_may_remove_an_older_one(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let older = enroll(&chat, &pool, &user).await;
    age(&chat, &pool, &older.row, 200).await;
    let younger = enroll(&chat, &pool, &user).await;
    let seasoned = age(&chat, &pool, &younger.row, 73).await;

    let (status, answer) = call(
        &chat,
        caller(&user, younger.session, Some(&seasoned)),
        "POST",
        &format!("/chat/devices/{}/not-me", older.keys.id),
        None,
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::FORBIDDEN, "chat_too_young"),
        "not me is only for devices that came later"
    );

    let (status, _) = call(
        &chat,
        caller(&user, younger.session, Some(&seasoned)),
        "DELETE",
        &format!("/chat/devices/{}", older.keys.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(status_of(&pool, older.keys.id).await, "revoked");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn not_me_revokes_a_newer_device_and_its_session(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let mine = enroll(&chat, &pool, &user).await;
    let mine_row = age(&chat, &pool, &mine.row, 1).await;
    let intruder = enroll(&chat, &pool, &user).await;
    let chat_id = conversation(&pool, "direct", &[user.as_str(), "7002"]).await;

    let (status, _) = call(
        &chat,
        caller(&user, mine.session, Some(&mine_row)),
        "POST",
        &format!("/chat/devices/{}/not-me", intruder.keys.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(status_of(&pool, intruder.keys.id).await, "revoked");
    assert!(!session_exists(&pool, intruder.session).await);
    assert!(
        rekey_pending(&pool, chat_id).await,
        "a revoked reader rotates the keys"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn only_an_older_seasoned_device_confirms_a_new_one(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let older = enroll(&chat, &pool, &user).await;
    let fresh_older = age(&chat, &pool, &older.row, 5).await;
    let newer = enroll(&chat, &pool, &user).await;
    let confirm = format!("/chat/devices/{}/confirm", newer.keys.id);

    let (status, answer) = call(
        &chat,
        caller(&user, older.session, Some(&fresh_older)),
        "POST",
        &confirm,
        None,
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::FORBIDDEN, "chat_too_young")
    );

    let (status, answer) = call(
        &chat,
        caller(&user, newer.session, Some(&newer.row)),
        "POST",
        &format!("/chat/devices/{}/confirm", older.keys.id),
        None,
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::FORBIDDEN, "chat_too_young")
    );

    let seasoned = age(&chat, &pool, &older.row, 100).await;
    let (status, _) = call(
        &chat,
        caller(&user, older.session, Some(&seasoned)),
        "POST",
        &confirm,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, listed) = call(
        &chat,
        caller(&user, newer.session, Some(&newer.row)),
        "GET",
        "/chat/devices",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let newer_view = listed
        .as_array()
        .expect("a list")
        .iter()
        .find(|device| device["id"] == json!(newer.keys.id))
        .expect("the newer device");
    assert_eq!(newer_view["history_unlocked"], json!(true));
    assert_eq!(newer_view["current"], json!(true));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn revoke_drops_the_device_from_caches_at_once(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let device = enroll(&chat, &pool, &user).await;
    let (status, _) = call(
        &chat,
        caller(&user, device.session, Some(&device.row)),
        "GET",
        "/chat/devices",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(chat.devices.row(device.keys.id).is_some());
    assert!(chat.devices.user(&user).is_some());

    let (status, _) = call(
        &chat,
        caller(&user, device.session, Some(&device.row)),
        "DELETE",
        "/chat/devices/me",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(chat.devices.user(&user).is_none());
    let cached = chat.devices.row(device.keys.id);
    assert!(cached.is_none_or(|row| row.status == "revoked"));
    let seen = lookup(&chat, device.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    assert_eq!(seen.status, "revoked");

    let (status, answer) = call(
        &chat,
        caller(&user, device.session, Some(&device.row)),
        "GET",
        "/chat/devices",
        None,
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::UNAUTHORIZED, "chat_device_revoked")
    );
    assert!(
        session_exists(&pool, device.session).await,
        "logging out of chat leaves the music session to the app"
    );

    let (status, answer) = call(
        &chat,
        caller(&user, device.session, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::UNAUTHORIZED, "chat_device_revoked"),
        "a revoked device never comes back with the same keys"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn leaving_chat_without_a_device_is_a_no_op(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let bare = session(&pool, &user, None).await;
    let (status, _) = call(
        &chat,
        caller(&user, bare, None),
        "DELETE",
        "/chat/devices/me",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn another_users_device_is_not_found(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let mine = enroll(&chat, &pool, &user).await;
    let theirs = enroll(&chat, &pool, "7002").await;
    for (method, path) in [
        ("DELETE", format!("/chat/devices/{}", theirs.keys.id)),
        ("POST", format!("/chat/devices/{}/confirm", theirs.keys.id)),
        ("POST", format!("/chat/devices/{}/not-me", theirs.keys.id)),
    ] {
        let (status, answer) = call(
            &chat,
            caller(&user, mine.session, Some(&mine.row)),
            method,
            &path,
            None,
        )
        .await;
        assert_eq!(
            (status, error(&answer)),
            (StatusCode::NOT_FOUND, "chat_not_found"),
            "{method} {path}"
        );
    }
    assert_eq!(status_of(&pool, theirs.keys.id).await, "active");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn touch_writes_last_seen_at_most_every_ten_minutes(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let device = enroll(&chat, &pool, &user).await;
    touch(&chat, &device.row).await.expect("a quiet touch");
    let cached = lookup(&chat, device.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    assert_eq!(cached.last_seen_at, device.row.last_seen_at);

    sqlx::query(
        "UPDATE chat.devices SET last_seen_at = now() - interval '11 minutes' WHERE id = $1",
    )
    .bind(device.keys.id)
    .execute(&pool)
    .await
    .expect("an idle device");
    chat.devices.forget(&user, &[device.keys.id]);
    let idle = lookup(&chat, device.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    touch(&chat, &idle).await.expect("a touch");
    assert!(chat.devices.row(device.keys.id).is_none());
    let seen = lookup(&chat, device.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    assert!(seen.last_seen_at > idle.last_seen_at);
}

async fn devices_delta(chat: &ChatState, user: &str) -> Option<f64> {
    let mut conn = chat.redis.get().await.expect("a redis connection");
    redis::cmd("ZSCORE")
        .arg(rt_keys::deltas(user))
        .arg(Change::Devices.member())
        .query_async(&mut conn)
        .await
        .expect("a score")
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn every_device_change_moves_the_devices_delta(pool: PgPool) {
    let chat = state(&pool);
    let user = format!("dev-{}", Uuid::now_v7().simple());
    let older = enroll(&chat, &pool, &user).await;
    let registered = devices_delta(&chat, &user)
        .await
        .expect("a register leaves a delta");
    let older_row = age(&chat, &pool, &older.row, 100).await;
    let newer = enroll(&chat, &pool, &user).await;
    let added = devices_delta(&chat, &user).await.expect("a delta");
    assert!(added > registered);

    let (status, _) = call(
        &chat,
        caller(&user, older.session, Some(&older_row)),
        "POST",
        &format!("/chat/devices/{}/confirm", newer.keys.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let confirmed = devices_delta(&chat, &user).await.expect("a delta");
    assert!(confirmed > added);

    let (status, _) = call(
        &chat,
        caller(&user, older.session, Some(&older_row)),
        "POST",
        &format!("/chat/devices/{}/not-me", newer.keys.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let disowned = devices_delta(&chat, &user).await.expect("a delta");
    assert!(disowned > confirmed);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn five_new_devices_a_day_and_a_relogin_is_not_one(pool: PgPool) {
    let chat = state(&pool);
    let user = format!("dev-{}", Uuid::now_v7().simple());
    fresh_device_budget(&chat, &user).await;
    let mut first = None;
    for _ in 0..5 {
        let keys = DeviceKeys::new();
        let session = session(&pool, &user, Some(keys.bind())).await;
        let (status, answer) = call(
            &chat,
            caller(&user, session, None),
            "POST",
            "/chat/devices",
            Some(desktop()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        first.get_or_insert((keys, session));
    }
    let extra = DeviceKeys::new();
    let extra_session = session(&pool, &user, Some(extra.bind())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, extra_session, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(
        (status, error(&answer)),
        (StatusCode::TOO_MANY_REQUESTS, "chat_request_limit")
    );

    let (keys, _) = first.expect("a first device");
    let relogin = session(&pool, &user, Some(keys.bind())).await;
    let (status, answer) = call(
        &chat,
        caller(&user, relogin, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let known: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat.users WHERE user_id = $1)")
            .bind(&user)
            .fetch_one(&pool)
            .await
            .expect("a probe");
    assert!(known, "a register records the chat user");
}
