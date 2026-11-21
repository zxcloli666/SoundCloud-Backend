use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::fake_store::{FakeStore, media_on};
use crate::modules::chat::media::rows::usage;
use crate::modules::chat::media::support::{
    IMAGE, THUMB, call, id_of, image_body, redis, reserve, star, user, with_media,
};
use crate::modules::chat::test_support::chat_state;

const MOTION: u64 = 524_288;
const TOTAL: i64 = (IMAGE + THUMB + MOTION) as i64;

type Row = (Uuid, String, Option<Uuid>, String, String);

fn uploading(pool: &PgPool, fake: &FakeStore) -> Arc<ChatState> {
    let mut media = media_on(&[(1, fake)]);
    media.uploads = true;
    with_media(chat_state(pool.clone(), redis(), true), media)
}

async fn rows(pool: &PgPool) -> Vec<Row> {
    sqlx::query_as("SELECT id, kind, parent_id, object_key, status FROM chat.media ORDER BY kind")
        .fetch_all(pool)
        .await
        .expect("rows")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_motion_reserves_commits_lists_and_leaves_with_its_image(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = uploading(&pool, &fake);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let mut body = image_body(group);
    body["motion_bytes"] = json!(MOTION);
    let answer = reserve(&chat, &owner, body.clone()).await;
    let root = id_of(&answer, "id");
    let motion: Uuid = serde_json::from_value(answer["motion"]["id"].clone()).expect("a motion");
    assert_eq!(
        answer["motion"]["put"]["headers"]["content-length"],
        MOTION.to_string()
    );
    assert_eq!(answer["used"].as_i64(), Some(TOTAL));
    let again = reserve(&chat, &owner, body).await;
    assert_eq!(again["motion"]["id"], answer["motion"]["id"]);
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));

    let reserved = rows(&pool).await;
    let kinds: Vec<&str> = reserved.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(kinds, ["image", "motion", "thumb"]);
    assert_eq!((reserved[1].0, reserved[1].2), (motion, Some(root)));
    for (row, size) in reserved.iter().zip([IMAGE, MOTION, THUMB]) {
        fake.put(&row.3, vec![7; size as usize]);
    }
    let commit = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{root}/commit"),
        None,
    )
    .await;
    assert_eq!((commit.0, commit.1["bytes"].as_i64()), (200, Some(TOTAL)));
    let committed = rows(&pool).await;
    assert!(committed.iter().all(|row| row.4 == "uploaded"));
    for ((before, after), size) in reserved.iter().zip(&committed).zip([IMAGE, MOTION, THUMB]) {
        assert_ne!(before.3, after.3);
        assert_eq!(
            fake.object(&after.3).map(|bytes| bytes.len() as u64),
            Some(size)
        );
    }
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));

    let mine = call(&chat, &owner, "GET", "/chat/media/mine", None).await;
    let items = mine.1["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{mine:?}");
    assert_eq!(items[0]["bytes"].as_i64(), Some(TOTAL));

    let deleted = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"ids": [root]})),
    )
    .await;
    assert_eq!(deleted.0, 200, "{deleted:?}");
    assert!(rows(&pool).await.iter().all(|row| row.4 == "deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}
