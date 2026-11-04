use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::{dm, group};
use crate::modules::chat::media::limits::QUOTA_BYTES;
use crate::modules::chat::media::rows::usage;
use crate::modules::chat::media::support::{
    AVATAR, IMAGE, Rig, StoreSpec, THUMB, avatar_body, bench, bench_with, call, error, id_of,
    image_body, live_room, reserve, star, unstar, user,
};
use crate::modules::chat::model::MemberState;

type ReservedRow = (Uuid, i16, String, String, Uuid, Option<Uuid>, Option<Uuid>);

const TOTAL: i64 = (IMAGE + THUMB) as i64;

fn two_stores(write: u16) -> Rig {
    let mut rig = Rig::offline();
    rig.stores.push(StoreSpec {
        id: 2,
        bucket: "scd-chat-2".to_owned(),
        cap_gb: 200,
    });
    rig.write = write;
    rig
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_reserve_lands_on_the_write_store_with_random_keys_and_signed_puts(pool: PgPool) {
    let chat = bench_with(&pool, &two_stores(2));
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let body = image_body(group);
    let answer = reserve(&chat, &owner, body.clone()).await;
    let root = id_of(&answer, "id");
    let thumb: Uuid = serde_json::from_value(answer["thumb"]["id"].clone()).expect("a thumb id");
    let put = answer["put"]["url"].as_str().expect("a put url");
    assert!(
        put.starts_with("https://s3.scnative.dev/scd-chat-2/m/"),
        "{put}"
    );
    assert_eq!(
        answer["put"]["headers"]["content-length"],
        IMAGE.to_string()
    );
    assert_eq!(
        answer["thumb"]["put"]["headers"]["content-length"],
        THUMB.to_string()
    );
    assert_eq!(
        (answer["used"].as_i64(), answer["expires_in_s"].as_i64()),
        (Some(TOTAL), Some(600))
    );
    assert_eq!(answer["quota"], QUOTA_BYTES);
    let rows: Vec<ReservedRow> = sqlx::query_as(
        "SELECT id, store, object_key, status, origin_conversation, parent_id, upload_id
         FROM chat.media ORDER BY kind",
    )
    .fetch_all(&pool)
    .await
    .expect("rows");
    let upload: Uuid = serde_json::from_value(body["upload_id"].clone()).expect("an upload id");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].0, rows[0].5, rows[0].6),
        (root, None, Some(upload))
    );
    assert_eq!((rows[1].0, rows[1].5), (thumb, Some(root)));
    for row in &rows {
        assert_eq!((row.1, row.3.as_str(), row.4), (2, "reserved", group));
        assert!(row.2.len() == 34 && row.2.starts_with("m/"), "{}", row.2);
        assert!(
            row.2[2..].chars().all(|c| c.is_ascii_hexdigit()),
            "{}",
            row.2
        );
        assert!(!put.contains(&row.2) || row.0 == root);
    }
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));
    let avatar = reserve(&chat, &owner, avatar_body(group)).await;
    assert!(avatar.get("thumb").is_none(), "{avatar}");
    assert_eq!(avatar["used"].as_i64(), Some(TOTAL + AVATAR as i64));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_replayed_upload_id_answers_the_same_ids_and_charges_nothing(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let body = image_body(group);
    let first = reserve(&chat, &owner, body.clone()).await;
    for _ in 0..40 {
        let again = reserve(&chat, &owner, body.clone()).await;
        assert_eq!(
            (again["id"].clone(), again["thumb"]["id"].clone()),
            (first["id"].clone(), first["thumb"]["id"].clone())
        );
        assert_eq!(again["used"].as_i64(), Some(TOTAL));
    }
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));
    reserve(&chat, &owner, image_body(group)).await;
    let mut resized = body.clone();
    resized["bytes"] = json!(IMAGE + 4096);
    let refused = call(&chat, &owner, "POST", "/chat/media/reserve", Some(resized)).await;
    assert_eq!(error(&refused), (400, "chat_invalid"));
    let mut moved = body.clone();
    moved["conversation_id"] = json!(Uuid::new_v4());
    let refused = call(&chat, &owner, "POST", "/chat/media/reserve", Some(moved)).await;
    assert_eq!(error(&refused), (400, "chat_invalid"));
    unstar(&pool, &owner).await;
    let fresh = bench(&pool);
    let lapsed = call(&fresh, &owner, "POST", "/chat/media/reserve", Some(body)).await;
    assert_eq!(error(&lapsed), (403, "chat_media_star_required"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_checks_answer_in_contract_order(pool: PgPool) {
    let mut paused = Rig::offline().config();
    paused.uploads = false;
    let off = crate::modules::chat::media::support::with_media(
        crate::modules::chat::test_support::chat_state(
            pool.clone(),
            crate::modules::chat::media::support::redis(),
            true,
        ),
        paused,
    );
    let owner = user();
    let free = user();
    let stranger = user();
    star(&pool, &owner).await;
    star(&pool, &stranger).await;
    let group = group(&pool, &owner, &[(&free, MemberState::Active)]).await;
    let answer = call(
        &off,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (503, "chat_media_uploads_paused"));
    let mut full = Rig::offline();
    full.stores[0].cap_gb = 1;
    let chat = bench_with(&pool, &full);
    let answer = call(
        &chat,
        &free,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_media_star_required"));
    let answer = call(
        &chat,
        &stranger,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_not_member"));
    reserve(&chat, &owner, image_body(group)).await;
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, bytes, status)
         VALUES ($1, 1, 'm/filler', 'filler', $2, 'image', 1073741824, 'attached')",
    )
    .bind(Uuid::new_v4())
    .bind(group)
    .execute(&pool)
    .await
    .expect("a filler row");
    let chat = bench_with(&pool, &full);
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (503, "chat_media_uploads_paused"));
    let mut nowhere = Rig::offline();
    nowhere.write = 3;
    let chat = bench_with(&pool, &nowhere);
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(image_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (503, "chat_media_uploads_paused"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn avatars_go_only_to_groups_and_only_from_admins(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, member) = (user(), user());
    star(&pool, &owner).await;
    star(&pool, &member).await;
    let group = group(&pool, &owner, &[(&member, MemberState::Active)]).await;
    let direct = dm(&pool, &owner, &member, MemberState::Active).await;
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(avatar_body(direct)),
    )
    .await;
    assert_eq!(error(&answer), (400, "chat_invalid"));
    let answer = call(
        &chat,
        &member,
        "POST",
        "/chat/media/reserve",
        Some(avatar_body(group)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_not_member"));
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/reserve",
        Some(avatar_body(Uuid::new_v4())),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_not_member"));
    reserve(&chat, &owner, avatar_body(group)).await;
    reserve(&chat, &member, image_body(direct)).await;
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn room_images_and_backdrops_reach_the_room_gates(pool: PgPool) {
    let chat = bench(&pool);
    let (host, guest) = (user(), user());
    star(&pool, &host).await;
    star(&pool, &guest).await;
    let room = live_room(&chat, &host, &[&guest]).await;
    let image = reserve(&chat, &guest, image_body(room.conversation)).await;
    assert!(image["thumb"]["id"].is_string(), "{}", room.code);
    let backdrop = json!({
        "upload_id": Uuid::new_v4(), "conversation_id": room.conversation,
        "kind": "backdrop", "bytes": 614_400
    });
    let reserved = reserve(&chat, &host, backdrop.clone()).await;
    assert!(reserved.get("thumb").is_none(), "{reserved}");
    let mut by_guest = backdrop.clone();
    by_guest["upload_id"] = json!(Uuid::new_v4());
    let answer = call(&chat, &guest, "POST", "/chat/media/reserve", Some(by_guest)).await;
    assert_eq!(error(&answer), (403, "chat_not_member"));
    let plain_host = user();
    let plain = live_room(&chat, &plain_host, &[&guest]).await;
    let answer = call(
        &chat,
        &guest,
        "POST",
        "/chat/media/reserve",
        Some(image_body(plain.conversation)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_room_star_required"));
    let mut oversized = backdrop.clone();
    oversized["bytes"] = json!(614_400 + 4096);
    let answer = call(&chat, &host, "POST", "/chat/media/reserve", Some(oversized)).await;
    assert_eq!(error(&answer), (400, "chat_invalid"));
    let mut with_thumb = backdrop;
    with_thumb["thumb_bytes"] = json!(4096);
    let answer = call(
        &chat,
        &host,
        "POST",
        "/chat/media/reserve",
        Some(with_thumb),
    )
    .await;
    assert_eq!(error(&answer), (400, "chat_invalid"));
    let free = user();
    let answer = call(
        &chat,
        &free,
        "POST",
        "/chat/media/reserve",
        Some(image_body(room.conversation)),
    )
    .await;
    assert_eq!(error(&answer), (403, "chat_media_star_required"));
}
