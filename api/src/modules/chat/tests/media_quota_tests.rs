use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::limits::{DAY_BYTES_MAX, QUOTA_BYTES};
use crate::modules::chat::media::rows::{set_usage, usage};
use crate::modules::chat::media::support::{
    IMAGE, Rig, THUMB, bench, bench_with, call, error, image_body, reserve, star, user,
};
use crate::modules::chat::quota;

const TOTAL: i64 = (IMAGE + THUMB) as i64;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn thirty_reserves_in_ten_minutes_then_the_rate_stops_them(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    for _ in 0..30 {
        reserve(&chat, &owner, image_body(group)).await;
    }
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (429, "chat_send_limited"));
    assert_eq!(usage(&pool, &owner).await.1, 30 * TOTAL);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_daily_byte_budget_and_the_quota_refuse_without_charging(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    set_usage(&pool, &owner, 0, DAY_BYTES_MAX as i64 - TOTAL + 4096).await;
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (429, "chat_send_limited"));
    sqlx::query("UPDATE chat.usage SET day = current_date - 1 WHERE user_id = $1")
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("yesterday");
    reserve(&chat, &owner, image_body(group)).await;
    let near = QUOTA_BYTES as i64 - 2 * TOTAL + 4096;
    set_usage(&pool, &owner, near, 0).await;
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_quota_exceeded"));
    assert_eq!(answer.1["used"].as_i64(), Some(near + TOTAL));
    assert_eq!(answer.1["quota"], QUOTA_BYTES);
    assert_eq!(usage(&pool, &owner).await, (near, TOTAL, 0));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.media")
        .fetch_one(&pool)
        .await
        .expect("a count");
    assert_eq!(rows, 2);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn fifty_parallel_reserves_never_pass_the_quota(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let room_for = 7;
    set_usage(&pool, &owner, QUOTA_BYTES as i64 - room_for * TOTAL, 0).await;
    let calls = (0..50).map(|_| {
        let chat = chat.clone();
        let owner = owner.clone();
        tokio::spawn(async move {
            call(
                &chat,
                &owner,
                "POST",
                "/chat/media/reserve",
                Some(image_body(group)),
            )
            .await
        })
    });
    let answers = futures::future::join_all(calls).await;
    let mut granted = 0;
    for answer in answers {
        let answer = answer.expect("a task");
        match error(&answer) {
            (200, _) => granted += 1,
            (403, "chat_quota_exceeded") | (429, "chat_send_limited") => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(granted, room_for);
    let (bytes, reserved, _) = usage(&pool, &owner).await;
    assert_eq!(bytes + reserved, QUOTA_BYTES as i64);
    let roots: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.media WHERE kind = 'image'")
        .fetch_one(&pool)
        .await
        .expect("a count");
    assert_eq!(roots, room_for);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn fifty_parallel_quota_charges_stop_at_the_quota(pool: PgPool) {
    let owner = user();
    let total: u64 = 32 << 20;
    set_usage(&pool, &owner, QUOTA_BYTES as i64 - 3 * total as i64, 0).await;
    let tasks = (0..50).map(|_| {
        let pool = pool.clone();
        let owner = owner.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("a tx");
            let charged = quota::reserve(&mut tx, &owner, total).await;
            tx.commit().await.expect("a commit");
            charged.is_ok()
        })
    });
    let granted = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter(|done| *done.as_ref().expect("a task"))
        .count();
    assert_eq!(granted, 3);
    let (bytes, reserved, _) = usage(&pool, &owner).await;
    assert_eq!(
        (bytes + reserved, reserved),
        (QUOTA_BYTES as i64, 3 * total as i64)
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_config_media_block_follows_the_switch_the_cap_and_star(pool: PgPool) {
    let owner = user();
    star(&pool, &owner).await;
    let open = crate::modules::chat::media::view::config(&bench(&pool), &owner)
        .await
        .expect("a view");
    assert!(open.uploads && open.star);
    assert_eq!(
        (open.quota, open.used, open.limits.restore_window_s),
        (QUOTA_BYTES, 0, 15)
    );
    let group = group(&pool, &owner, &[]).await;
    let chat = bench(&pool);
    reserve(&chat, &owner, image_body(group)).await;
    let config = call(&chat, &owner, "GET", "/chat/config", None).await;
    assert_eq!(
        config.1["media"]["used"].as_i64(),
        Some(TOTAL),
        "{config:?}"
    );
    assert_eq!(config.1["media"]["uploads"], true);
    assert_eq!(config.1["media"]["limits"]["room_album_max"], 4);
    let mut full = Rig::offline();
    full.stores[0].cap_gb = 0;
    let closed = crate::modules::chat::media::view::config(&bench_with(&pool, &full), &owner)
        .await
        .expect("a view");
    assert!(!closed.uploads && closed.star);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn parallel_reserves_of_one_upload_id_make_one_reservation(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let body = image_body(group);
    let calls = (0..12).map(|_| {
        let (chat, owner, body) = (chat.clone(), owner.clone(), body.clone());
        tokio::spawn(
            async move { call(&chat, &owner, "POST", "/chat/media/reserve", Some(body)).await },
        )
    });
    let answers = futures::future::join_all(calls).await;
    let ids: std::collections::HashSet<String> = answers
        .into_iter()
        .map(|answer| {
            let answer = answer.expect("a task");
            assert_eq!(answer.0, 200, "{answer:?}");
            answer.1["id"].as_str().expect("an id").to_owned()
        })
        .collect();
    assert_eq!(ids.len(), 1);
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.media")
        .fetch_one(&pool)
        .await
        .expect("a count");
    assert_eq!(rows, 2);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_rate_answers_before_the_cap_and_the_cap_before_the_quota(pool: PgPool) {
    let owner = user();
    star(&pool, &owner).await;
    let mine = group(&pool, &owner, &[]).await;
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, bytes, status)
         VALUES ($1, 1, 'm/filler', 'filler', $2, 'image', 1073741824, 'attached')",
    )
    .bind(Uuid::new_v4())
    .bind(mine)
    .execute(&pool)
    .await
    .expect("a filler row");
    set_usage(&pool, &owner, QUOTA_BYTES as i64, 0).await;
    let mut full = Rig::offline();
    full.stores[0].cap_gb = 1;
    let chat = bench_with(&pool, &full);
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(mine)),
    )
    .await;
    assert_eq!(error(&answer), (503, "chat_media_uploads_paused"));
    for _ in 0..29 {
        call(
            &chat,
            &owner,
            "POST",
            "/chat/media/reserve",
            Some(image_body(mine)),
        )
        .await;
    }
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(mine)),
    )
    .await;
    assert_eq!(error(&answer), (429, "chat_send_limited"));
    let open = bench(&pool);
    let other = user();
    star(&pool, &other).await;
    set_usage(&pool, &other, QUOTA_BYTES as i64, 0).await;
    let theirs = group(&pool, &other, &[]).await;
    let answer = call(
        &open,
        &other,
        "POST",
        "/chat/media/reserve",
        Some(image_body(theirs)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_quota_exceeded"));
}
