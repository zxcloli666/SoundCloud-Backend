use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::dm;
use crate::modules::chat::media::rows::{Seed, refs as refs_of, seed};
use crate::modules::chat::media::support::{image_body, star};
use crate::modules::chat::model::MemberState;
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{
    OWNER, age_membership, ask, audit_count, bench, error, keys_status, moderator, open, post,
    post_body, posted, reset, send, serial, slow_mode, user,
};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_muted_member_gets_chat_muted_with_the_end_of_the_mute(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let me = user();
    open(&chat, &me).await;
    age_membership(&pool, &me, 60).await;
    posted(&chat, &me).await;
    let muted = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/mutes/{me}"),
        Some(json!({"seconds": 600, "reason": "spam"})),
    )
    .await;
    assert_eq!(muted.0, 200, "{muted:?}");
    let until = muted.1["until"].as_i64().expect("an end");
    let now = chrono::Utc::now().timestamp_millis();
    assert!(
        (599_000..=601_000).contains(&(until - now)),
        "{until} {now}"
    );
    let refused = post(&chat, &me).await;
    assert_eq!(error(&refused), (403, "chat_muted"));
    assert_eq!(refused.1["until"], until);
    let lifted = ask(
        &chat,
        OWNER,
        "DELETE",
        &format!("/chat/public/mutes/{me}"),
        None,
    )
    .await;
    assert_eq!(lifted.0, 204, "{lifted:?}");
    assert_eq!(post(&chat, &me).await.0, 200);
    assert_eq!(audit_count(&pool, "mute").await, 1);
    assert_eq!(audit_count(&pool, "unmute").await, 1);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_mute_is_one_of_three_lengths(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    for seconds in [0, 60, 7_200, 604_800] {
        let refused = ask(
            &chat,
            OWNER,
            "PUT",
            &format!("/chat/public/mutes/{me}"),
            Some(json!({"seconds": seconds})),
        )
        .await;
        assert_eq!(error(&refused), (400, "chat_invalid"), "{seconds}");
    }
    for seconds in [3_600, 86_400] {
        let muted = ask(
            &chat,
            OWNER,
            "PUT",
            &format!("/chat/public/mutes/{me}"),
            Some(json!({"seconds": seconds})),
        )
        .await;
        assert_eq!(muted.0, 200, "{muted:?}");
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn slow_mode_holds_members_and_lets_moderators_and_the_owner_through(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (member, helper) = (user(), user());
    for who in [&member, &helper] {
        open(&chat, who).await;
        age_membership(&pool, who, 60).await;
    }
    moderator(&chat, &helper).await;
    assert_eq!(post(&chat, &member).await.0, 200);
    let held = post(&chat, &member).await;
    assert_eq!(error(&held), (429, "chat_slow_mode"));
    let retry = held.1["retry_ms"].as_u64().expect("a wait");
    assert!((1..=5_000).contains(&retry), "{retry}");
    for who in [helper.as_str(), OWNER] {
        assert_eq!(post(&chat, who).await.0, 200);
        assert_eq!(post(&chat, who).await.0, 200);
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_retry_of_a_stored_post_is_answered_even_while_slow_mode_holds(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    open(&chat, &me).await;
    age_membership(&pool, &me, 60).await;
    let body = post_body(&[]);
    let first = send(&chat, &me, body.clone()).await;
    assert_eq!(first.0, 200, "{first:?}");
    let again = send(&chat, &me, body).await;
    assert_eq!(again.1["seq"], first.1["seq"]);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_newcomer_posts_short_for_ten_minutes_then_like_everyone(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let me = user();
    open(&chat, &me).await;
    let long = json!({
        "client_id": Uuid::now_v7(), "class": "post", "epoch": 0,
        "body": crate::modules::chat::b64::encode(&[7u8; 700]),
    });
    let refused = send(&chat, &me, long.clone()).await;
    assert_eq!(error(&refused), (429, "chat_send_limited"));
    age_membership(&pool, &me, 11).await;
    let allowed = send(&chat, &me, long).await;
    assert_eq!(allowed.0, 200, "{allowed:?}");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_banned_member_can_neither_read_nor_post_and_an_unban_restores_them(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let me = user();
    open(&chat, &me).await;
    age_membership(&pool, &me, 60).await;
    posted(&chat, &me).await;
    let banned = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/bans/{me}"),
        Some(json!({"reason": "ads"})),
    )
    .await;
    assert_eq!(banned.0, 204, "{banned:?}");
    let denied = (403, "chat_not_member");
    let read = ask(
        &chat,
        &me,
        "GET",
        &format!("/chat/conversations/{ID}/messages"),
        None,
    )
    .await;
    assert_eq!(error(&read), denied);
    assert_eq!(error(&post(&chat, &me).await), denied);
    assert_eq!(error(&keys_status(&chat, &pool, &me).await), denied);
    let watch = ask(&chat, &me, "POST", "/chat/public/watch", None).await;
    assert_eq!(error(&watch), denied);
    let seen = open(&chat, &me).await;
    assert_eq!(seen["conversation"]["me"]["state"], "banned");
    let lifted = ask(
        &chat,
        OWNER,
        "DELETE",
        &format!("/chat/public/bans/{me}"),
        None,
    )
    .await;
    assert_eq!(lifted.0, 204, "{lifted:?}");
    assert_eq!(post(&chat, &me).await.0, 200);
    assert_eq!(audit_count(&pool, "ban").await, 1);
    assert_eq!(audit_count(&pool, "unban").await, 1);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_ban_reaches_someone_who_never_opened_the_chat(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let stranger = user();
    let banned = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/bans/{stranger}"),
        Some(json!({})),
    )
    .await;
    assert_eq!(banned.0, 204, "{banned:?}");
    let seen = open(&chat, &stranger).await;
    assert_eq!(seen["conversation"]["me"]["state"], "banned");
    assert_eq!(
        error(&post(&chat, &stranger).await),
        (403, "chat_not_member")
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn images_are_for_star_members_and_stay_inside_the_public_chat(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (free, paid, friend) = (user(), user(), user());
    for who in [&free, &paid] {
        open(&chat, who).await;
        age_membership(&pool, who, 60).await;
    }
    let refused = ask(
        &chat,
        &free,
        "POST",
        "/chat/media/reserve",
        Some(image_body(ID)),
    )
    .await;
    assert_eq!(error(&refused), (403, "chat_media_star_required"));
    star(&pool, &paid).await;
    let reserved = ask(
        &chat,
        &paid,
        "POST",
        "/chat/media/reserve",
        Some(image_body(ID)),
    )
    .await;
    assert_eq!(reserved.0, 200, "{reserved:?}");

    let (image, _) = seed(&pool, Seed::image(&paid, ID, "uploaded")).await;
    let posted = send(&chat, &paid, post_body(&[image])).await;
    assert_eq!(posted.0, 200, "{posted:?}");
    assert_eq!(
        refs_of(&pool, image).await,
        [(ID, posted.1["seq"].as_i64().unwrap())]
    );

    let direct = dm(&pool, &paid, &friend, MemberState::Active).await;
    let out = ask(
        &chat,
        &paid,
        "POST",
        &format!("/chat/conversations/{direct}/messages"),
        Some(post_body(&[image])),
    )
    .await;
    assert_eq!(error(&out), (409, "chat_media_missing"));

    let (elsewhere, _) = seed(&pool, Seed::image(&paid, direct, "uploaded")).await;
    let into = send(&chat, &paid, post_body(&[elsewhere])).await;
    assert_eq!(error(&into), (409, "chat_media_missing"));
    assert_eq!(audit_count(&pool, "delete").await, 0);
}
