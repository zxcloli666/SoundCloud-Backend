use axum::http::StatusCode;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::conversation_key;
use super::rotate::{ensure_current, rotate};
use crate::modules::chat::b64;
use crate::modules::chat::devices::test_support::{
    Enrolled, age, call, caller, conversation, enroll, fresh_user, rekey_pending, state,
};
use crate::modules::chat::model::DeviceRow;
use crate::modules::chat::{ChatCaller, ChatState};

const PEER: &str = "7102";

struct Opened {
    epoch: i32,
    sealed: Vec<u8>,
    ck: [u8; 32],
}

fn as_caller(device: &Enrolled, row: &DeviceRow) -> ChatCaller {
    caller(&row.user_id, device.session, Some(row))
}

async fn keys(
    chat: &std::sync::Arc<ChatState>,
    device: &Enrolled,
    row: &DeviceRow,
    conversation: Uuid,
    from_epoch: i32,
) -> (StatusCode, Value) {
    call(
        chat,
        as_caller(device, row),
        "GET",
        &format!("/chat/conversations/{conversation}/keys?from_epoch={from_epoch}"),
        None,
    )
    .await
}

fn opened(device: &Enrolled, conversation: Uuid, answer: &Value) -> Vec<Opened> {
    answer
        .as_array()
        .expect("a key list")
        .iter()
        .map(|key| {
            let epoch = i32::try_from(key["epoch"].as_i64().expect("an epoch")).expect("an epoch");
            let sealed = b64::decode(key["sealed"].as_str().expect("sealed")).expect("base64");
            let ck = sc_chat_wire::ck::open_for_device(
                &device.keys.box_secret,
                conversation.as_bytes(),
                u32::try_from(epoch).expect("an epoch"),
                device.keys.id.as_bytes(),
                &sealed,
            )
            .expect("the key opens with the device box key");
            Opened { epoch, sealed, ck }
        })
        .collect()
}

fn epochs(keys: &[Opened]) -> Vec<i32> {
    keys.iter().map(|key| key.epoch).collect()
}

async fn backdate_keys(pool: &PgPool, conversation: Uuid, hours: i32) {
    sqlx::query(
        "UPDATE chat.conversation_keys SET created_at = now() - $2::int * interval '1 hour' WHERE conversation_id = $1",
    )
    .bind(conversation)
    .bind(hours)
    .execute(pool)
    .await
    .expect("backdated keys");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_member_gets_the_current_key_sealed_to_its_box(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let chat_id = conversation(&pool, "direct", &[user.as_str(), PEER]).await;
    let device = enroll(&chat, &pool, &user).await;

    let (status, answer) = keys(&chat, &device, &device.row, chat_id, 0).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let first = opened(&device, chat_id, &answer);
    assert_eq!(
        epochs(&first),
        vec![1],
        "the pending rekey of a new device rotates first"
    );
    assert_eq!(
        conversation_key(&chat, chat_id, 1)
            .await
            .expect("the stored key"),
        first[0].ck
    );
    assert!(!rekey_pending(&pool, chat_id).await);

    let (_, again) = keys(&chat, &device, &device.row, chat_id, 0).await;
    let second = opened(&device, chat_id, &again);
    assert_eq!(epochs(&second), vec![1]);
    assert_eq!(second[0].ck, first[0].ck);
    assert_ne!(
        second[0].sealed, first[0].sealed,
        "every delivery seals with a fresh ephemeral key"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn only_members_get_keys(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let device = enroll(&chat, &pool, &user).await;
    let foreign = conversation(&pool, "direct", &[PEER, "7103"]).await;
    let left = conversation(&pool, "group", &[user.as_str(), PEER]).await;
    sqlx::query(
        "UPDATE chat.members SET state = 'kicked' WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(left)
    .bind(&user)
    .execute(&pool)
    .await
    .expect("a kick");
    let gone = conversation(&pool, "group", &[user.as_str(), PEER]).await;
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(gone)
        .execute(&pool)
        .await
        .expect("a shred");

    for (conversation, expected) in [
        (foreign, (StatusCode::FORBIDDEN, "chat_not_member")),
        (left, (StatusCode::FORBIDDEN, "chat_not_member")),
        (gone, (StatusCode::NOT_FOUND, "chat_not_found")),
        (Uuid::now_v7(), (StatusCode::NOT_FOUND, "chat_not_found")),
    ] {
        let (status, answer) = keys(&chat, &device, &device.row, conversation, 0).await;
        assert_eq!(
            (status, answer["error"].as_str().unwrap_or_default()),
            expected
        );
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.conversation_keys")
        .fetch_one(&pool)
        .await
        .expect("a count");
    assert_eq!(stored, 0, "a refused caller creates no key");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_history_gate_holds_until_an_older_device_confirms(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let old = enroll(&chat, &pool, &user).await;
    let old_row = age(&chat, &pool, &old.row, 300).await;
    let chat_id = conversation(&pool, "direct", &[user.as_str(), PEER]).await;
    let (_, before) = keys(&chat, &old, &old_row, chat_id, 0).await;
    assert_eq!(epochs(&opened(&old, chat_id, &before)), vec![0]);
    backdate_keys(&pool, chat_id, 200).await;

    let new = enroll(&chat, &pool, &user).await;
    assert!(rekey_pending(&pool, chat_id).await);
    let (status, locked) = keys(&chat, &new, &new.row, chat_id, 0).await;
    assert_eq!(status, StatusCode::OK, "{locked}");
    let locked = opened(&new, chat_id, &locked);
    assert_eq!(epochs(&locked), vec![1], "new messages flow at once");

    let (_, full) = keys(&chat, &old, &old_row, chat_id, 0).await;
    let full = opened(&old, chat_id, &full);
    assert_eq!(epochs(&full), vec![0, 1]);
    assert_eq!(full[1].ck, locked[0].ck);

    let (status, _) = call(
        &chat,
        as_caller(&old, &old_row),
        "POST",
        &format!("/chat/devices/{}/confirm", new.keys.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let confirmed = crate::modules::chat::devices::lookup(&chat, new.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    let (_, unlocked) = keys(&chat, &new, &confirmed, chat_id, 0).await;
    let unlocked = opened(&new, chat_id, &unlocked);
    assert_eq!(epochs(&unlocked), vec![0, 1]);
    assert_eq!(unlocked[0].ck, full[0].ck);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_history_gate_opens_by_itself_after_72_hours(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let old = enroll(&chat, &pool, &user).await;
    let old_row = age(&chat, &pool, &old.row, 300).await;
    let chat_id = conversation(&pool, "direct", &[user.as_str(), PEER]).await;
    keys(&chat, &old, &old_row, chat_id, 0).await;
    backdate_keys(&pool, chat_id, 200).await;

    let new = enroll(&chat, &pool, &user).await;
    let (_, locked) = keys(&chat, &new, &new.row, chat_id, 0).await;
    assert_eq!(epochs(&opened(&new, chat_id, &locked)), vec![1]);

    let almost = age(&chat, &pool, &new.row, 71).await;
    let (_, still) = keys(&chat, &new, &almost, chat_id, 0).await;
    assert_eq!(epochs(&opened(&new, chat_id, &still)), vec![1]);

    let waited = age(&chat, &pool, &new.row, 73).await;
    let (_, open) = keys(&chat, &new, &waited, chat_id, 0).await;
    assert_eq!(epochs(&opened(&new, chat_id, &open)), vec![0, 1]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_chat_joined_after_registration_gives_a_new_device_its_current_key(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let group = conversation(&pool, "group", &[PEER, "7103"]).await;
    ensure_current(&chat, group).await.expect("the first key");
    for _ in 0..2 {
        rotate(&chat, group).await.expect("a rotation");
    }
    backdate_keys(&pool, group, 200).await;

    let new = enroll(&chat, &pool, &user).await;
    sqlx::query(
        "INSERT INTO chat.members (conversation_id, user_id, state) VALUES ($1, $2, 'active')",
    )
    .bind(group)
    .bind(&user)
    .execute(&pool)
    .await
    .expect("a late join");
    assert!(!rekey_pending(&pool, group).await);

    let (status, answer) = keys(&chat, &new, &new.row, group, 0).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        epochs(&opened(&new, group, &answer)),
        vec![2],
        "new messages flow at once, older epochs wait for the gate"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_group_that_hides_history_starts_at_the_join_epoch(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let device = enroll(&chat, &pool, &user).await;
    let row = age(&chat, &pool, &device.row, 100).await;
    let group = conversation(&pool, "group", &[PEER, user.as_str()]).await;
    for _ in 0..3 {
        rotate(&chat, group).await.expect("a rotation");
    }
    sqlx::query(
        "UPDATE chat.conversations SET settings = '{\"history_visible\": false}' WHERE id = $1",
    )
    .bind(group)
    .execute(&pool)
    .await
    .expect("hidden history");
    sqlx::query(
        "UPDATE chat.members SET joined_epoch = 2 WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(group)
    .bind(&user)
    .execute(&pool)
    .await
    .expect("a join epoch");

    let (_, hidden) = keys(&chat, &device, &row, group, 0).await;
    assert_eq!(epochs(&opened(&device, group, &hidden)), vec![2, 3]);

    sqlx::query("UPDATE chat.conversations SET settings = '{}' WHERE id = $1")
        .bind(group)
        .execute(&pool)
        .await
        .expect("visible history");
    let (_, visible) = keys(&chat, &device, &row, group, 0).await;
    assert_eq!(epochs(&opened(&device, group, &visible)), vec![1, 2, 3]);
    let (_, newest) = keys(&chat, &device, &row, group, 3).await;
    assert_eq!(epochs(&opened(&device, group, &newest)), vec![3]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_pending_rekey_rotates_once_under_a_race(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let chat_id = conversation(&pool, "group", &[user.as_str(), PEER]).await;
    assert_eq!(ensure_current(&chat, chat_id).await, Ok(0));
    assert_eq!(ensure_current(&chat, chat_id).await, Ok(0));
    sqlx::query("UPDATE chat.conversations SET rekey = true WHERE id = $1")
        .bind(chat_id)
        .execute(&pool)
        .await
        .expect("a rekey");

    let (left, right) = tokio::join!(
        ensure_current(&chat, chat_id),
        ensure_current(&chat, chat_id)
    );
    assert_eq!((left, right), (Ok(1), Ok(1)));
    let state: (i32, bool, i32) = sqlx::query_as(
        "SELECT key_epoch, rekey, epoch_messages FROM chat.conversations WHERE id = $1",
    )
    .bind(chat_id)
    .fetch_one(&pool)
    .await
    .expect("a conversation");
    assert_eq!(state, (1, false, 0));
    assert_eq!(rotate(&chat, chat_id).await, Ok(2));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_small_answer_budget_pages_the_keys(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let device = enroll(&chat, &pool, &user).await;
    let row = age(&chat, &pool, &device.row, 100).await;
    let chat_id = conversation(&pool, "group", &[user.as_str(), PEER]).await;
    for _ in 0..5 {
        rotate(&chat, chat_id).await.expect("a rotation");
    }
    let mut pro = as_caller(&device, &row);
    pro.max_bytes = 400;
    let (status, answer) = call(
        &chat,
        pro,
        "GET",
        &format!("/chat/conversations/{chat_id}/keys?from_epoch=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(epochs(&opened(&device, chat_id, &answer)), vec![1, 2]);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_revoked_device_gets_no_keys(pool: PgPool) {
    let user = fresh_user();
    let chat = state(&pool);
    let device = enroll(&chat, &pool, &user).await;
    let chat_id = conversation(&pool, "direct", &[user.as_str(), PEER]).await;
    crate::modules::chat::devices::revoke(&chat, &device.row)
        .await
        .expect("a revoke");
    let (status, answer) = keys(&chat, &device, &device.row, chat_id, 0).await;
    assert_eq!(
        (status, answer["error"].as_str().unwrap_or_default()),
        (StatusCode::UNAUTHORIZED, "chat_device_revoked")
    );
    let (status, answer) = call(
        &chat,
        caller(&user, device.session, None),
        "GET",
        &format!("/chat/conversations/{chat_id}/keys"),
        None,
    )
    .await;
    assert_eq!(
        (status, answer["error"].as_str().unwrap_or_default()),
        (StatusCode::UNAUTHORIZED, "chat_device_revoked"),
        "keys need a registered device"
    );
}
