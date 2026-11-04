use std::path::Path;

use sqlx::PgPool;
use uuid::Uuid;

use super::{cap_bytes, get, missing, verify_rows, write};
use crate::modules::chat::media::MediaConfig;

fn config(stores: &str, write_to: &str) -> MediaConfig {
    let pairs = [
        ("CHAT_STORES", stores),
        ("CHAT_STORE_WRITE", write_to),
        ("CHAT_STORE_1_ENDPOINT", "https://s3.int.scnative.dev:8443"),
        ("CHAT_STORE_1_PRESIGN", "https://s3.scnative.dev"),
        ("CHAT_STORE_1_BUCKET", "scd-chat"),
        ("CHAT_STORE_1_ACCESS_KEY", "AKIA"),
        ("CHAT_STORE_1_CAP_GB", "200"),
        ("CHAT_STORE_2_ENDPOINT", "https://s3b.int.scnative.dev:8443"),
        ("CHAT_STORE_2_PRESIGN", "https://s3.scnative.dev"),
        ("CHAT_STORE_2_BUCKET", "scd-chat-2"),
        ("CHAT_STORE_2_ACCESS_KEY", "AKIB"),
    ];
    MediaConfig::parse(
        |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        },
        Path::new("/nonexistent"),
    )
}

#[test]
fn a_write_store_and_a_cap_come_from_the_config() {
    let config = config("1,2", "2");
    assert_eq!(write(&config).map(|store| store.id), Some(2));
    assert_eq!(get(&config, 1).map(cap_bytes), Some(200 << 30));
    assert_eq!(get(&config, 3).map(|store| store.id), None);
    assert_eq!(write(&self::config("1", "")).map(|store| store.id), None);
    assert_eq!(write(&self::config("1", "2")).map(|store| store.id), None);
}

#[test]
fn rows_naming_an_unlisted_store_are_reported() {
    let config = config("1", "1");
    assert_eq!(missing(&config, &[1, 2, 3]), vec![2, 3]);
    assert!(missing(&config, &[1]).is_empty());
}

async fn insert(pool: &PgPool, store: i16) {
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, bytes, status)
         VALUES ($1, $2, $3, '1', $1, 'image', 4096, 'uploaded')",
    )
    .bind(Uuid::new_v4())
    .bind(store)
    .bind(format!("m/{}", Uuid::new_v4().simple()))
    .execute(pool)
    .await
    .expect("a row");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn boot_fails_when_a_row_names_a_store_missing_from_config(pool: PgPool) {
    verify_rows(&pool, &config("1", "1"))
        .await
        .expect("no rows");
    insert(&pool, 1).await;
    insert(&pool, 1).await;
    verify_rows(&pool, &config("1", "1"))
        .await
        .expect("store 1 is listed");
    insert(&pool, 2).await;
    let error = verify_rows(&pool, &config("1", "1"))
        .await
        .expect_err("store 2 is not listed");
    assert!(error.to_string().contains("[2]"), "{error}");
    verify_rows(&pool, &config("1,2", "2"))
        .await
        .expect("both listed");
    assert!(verify_rows(&pool, &config("1", "2")).await.is_err());
}
