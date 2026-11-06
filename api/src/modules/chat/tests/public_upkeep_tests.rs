use std::sync::Arc;

use serde_json::json;
use sqlx::PgPool;

use crate::modules::chat::devices::test_support::{caller, enroll, fresh_user};
use crate::modules::chat::media::rows::{Seed, refs as refs_of, seed, status};
use crate::modules::chat::media::support::redis;
use crate::modules::chat::moderation::evidence;
use crate::modules::chat::moderation::testing::{
    TestKeys, conversation_key, post as stored, state_with,
};
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{
    OWNER, bench, error, open, post, post_body, reset, send, serial, slow_mode, user,
};
use crate::modules::chat::reports::{FileReport, file_with, load};
use crate::modules::chat::sweeps::public_chat as sweep;

async fn epoch_of(pool: &PgPool) -> i32 {
    sqlx::query_scalar("SELECT key_epoch FROM chat.conversations WHERE id = $1")
        .bind(ID)
        .fetch_one(pool)
        .await
        .expect("an epoch")
}

async fn insert_posts(pool: &PgPool, count: i64, days_old: i32) -> Vec<i64> {
    sqlx::query_scalar(
        "WITH bump AS (
             UPDATE chat.conversations SET last_seq = last_seq + $2 WHERE id = $1 RETURNING last_seq
         )
         INSERT INTO chat.messages (conversation_id, seq, client_id, sender_user, class, epoch, body, created_at)
         SELECT $1, bump.last_seq - $2 + n, gen_random_uuid(), $4, 'post', 0, '\\x01'::bytea,
                now() - $3::int * interval '1 day'
         FROM bump, generate_series(1, $2) AS n
         RETURNING seq",
    )
    .bind(ID)
    .bind(count)
    .bind(days_old)
    .bind(OWNER)
    .fetch_all(pool)
    .await
    .expect("posts")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_server_key_is_sealed_to_a_fresh_device_without_a_history_gate(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = fresh_user();
    let device = enroll(&chat, &pool, &me).await;
    open(&chat, &me).await;
    let mut row = device.row.clone();
    row.caps
        .push(crate::modules::chat::devices::CAP_PUBLIC_CHAT.to_owned());
    let who = caller(&me, device.session, Some(&row));
    let keys = crate::modules::chat::devices::test_support::call(
        &chat,
        who,
        "GET",
        &format!("/chat/conversations/{ID}/keys"),
        None,
    )
    .await;
    assert_eq!(keys.0, axum::http::StatusCode::OK, "{keys:?}");
    let sealed = keys.1.as_array().expect("keys");
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0]["epoch"], 0);
    let plain = crate::modules::chat::keys::conversation_key(&chat, ID, 0)
        .await
        .expect("the stored key");
    assert_eq!(plain.len(), 32);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_key_turns_over_weekly_and_not_by_message_count(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let me = user();
    open(&chat, &me).await;
    sqlx::query("UPDATE chat.conversations SET epoch_messages = 2000000000 WHERE id = $1")
        .bind(ID)
        .execute(&pool)
        .await
        .expect("a busy epoch");
    assert_eq!(post(&chat, OWNER).await.0, 200);
    assert_eq!(epoch_of(&pool).await, 0);
    sqlx::query(
        "UPDATE chat.conversations SET epoch_started_at = now() - interval '8 days' WHERE id = $1",
    )
    .bind(ID)
    .execute(&pool)
    .await
    .expect("an old epoch");
    let stale = post(&chat, OWNER).await;
    assert_eq!(error(&stale), (409, "chat_epoch_stale"));
    assert_eq!(epoch_of(&pool).await, 1);
    let mut body = post_body(&[]);
    body["epoch"] = json!(1);
    assert_eq!(send(&chat, OWNER, body).await.0, 200);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_sweep_removes_posts_after_seven_days_in_paced_batches(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let old = insert_posts(&pool, 1_200, 8).await;
    let fresh = insert_posts(&pool, 3, 1).await;
    let (image, _) = seed(&pool, Seed::image(OWNER, ID, "attached")).await;
    sqlx::query("INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, $3)")
        .bind(image)
        .bind(ID)
        .bind(old[0])
        .execute(&pool)
        .await
        .expect("a ref");
    let deleted = sweep::run_once(&chat).await.expect("a sweep");
    assert_eq!(deleted, 1_200);
    let left: Vec<i64> =
        sqlx::query_scalar("SELECT seq FROM chat.messages WHERE conversation_id = $1 ORDER BY seq")
            .bind(ID)
            .fetch_all(&pool)
            .await
            .expect("seqs");
    assert_eq!(left, fresh);
    assert!(refs_of(&pool, image).await.is_empty());
    assert_ne!(status(&pool, image).await.as_deref(), Some("attached"));
    assert_eq!(sweep::run_once(&chat).await.expect("a quiet sweep"), 0);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_sweep_drops_lapsed_mutes(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    sqlx::query("INSERT INTO chat.public_mutes (user_id, until, by) VALUES ('1', now() - interval '1 minute', $1), ('2', now() + interval '1 hour', $1)")
        .bind(OWNER)
        .execute(&pool)
        .await
        .expect("mutes");
    sweep::run_once(&chat).await.expect("a sweep");
    let left: Vec<String> = sqlx::query_scalar("SELECT user_id FROM chat.public_mutes")
        .fetch_all(&pool)
        .await
        .expect("mutes");
    assert_eq!(left, ["2"]);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_report_from_the_public_chat_is_sealed_like_any_other(pool: PgPool) {
    let keys = TestKeys::default();
    let mut chat = state_with(pool.clone(), redis(), None);
    Arc::get_mut(&mut chat)
        .expect("a fresh state")
        .config
        .public = true;
    let ck = conversation_key(&pool, &keys, ID).await;
    let (reporter, target) = (user(), user());
    for who in [&reporter, &target] {
        sqlx::query("INSERT INTO chat.members (conversation_id, user_id, state, accepted) VALUES ($1, $2, 'active', true)")
            .bind(ID)
            .bind(who)
            .execute(&pool)
            .await
            .expect("a member");
    }
    stored(&pool, ID, &ck, &reporter, "hello").await;
    let bad = stored(&pool, ID, &ck, &target, "you are all idiots").await;
    let input = FileReport {
        conversation_id: ID,
        seq_from: bad,
        seq_to: bad,
        reason: "harassment".to_owned(),
        comment: None,
    };
    let filed = file_with(&chat, &keys, &reporter, &input)
        .await
        .expect("a report");
    assert!(!filed.duplicate);
    let row = load(&chat, filed.id).await.expect("read").expect("a row");
    assert_eq!(row.target_user, target);
    let bundle = evidence::open(&keys, 1, &row.evidence.expect("evidence")).expect("opens");
    assert_eq!(evidence::lines(&bundle).len(), 2);
    let outsider = user();
    let refused = file_with(&chat, &keys, &outsider, &input).await;
    assert_eq!(
        refused.err(),
        Some(crate::modules::chat::ChatError::NotMember)
    );
}
