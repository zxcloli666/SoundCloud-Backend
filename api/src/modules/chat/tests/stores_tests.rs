use std::path::Path;

use sqlx::PgPool;
use uuid::Uuid;

use super::{cap_bytes, filled, get, missing, verify_rows, write};
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::media::support::{Rig, bench_with};

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
    .expect("a media config")
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
    insert_as(pool, store, "uploaded", 4096).await;
}

async fn insert_as(pool: &PgPool, store: i16, status: &str, bytes: i64) {
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, bytes, status)
         VALUES ($1, $2, $3, '1', $1, 'image', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(store)
    .bind(format!("m/{}", Uuid::new_v4().simple()))
    .bind(bytes)
    .bind(status)
    .execute(pool)
    .await
    .expect("a row");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_fill_counts_every_row_with_an_object_or_a_promise_and_skips_lost_ones(pool: PgPool) {
    let statuses = [
        "reserved", "uploaded", "attached", "held", "deleting", "lost",
    ];
    for (index, status) in statuses.into_iter().enumerate() {
        insert_as(&pool, 1, status, 4096 << index).await;
    }
    insert_as(&pool, 2, "attached", 1 << 30).await;
    let chat = bench_with(&pool, &Rig::offline());
    let expected = (0..5).map(|index| 4096u64 << index).sum::<u64>();
    assert_eq!(filled(&chat, 1).await, Ok(expected));
    insert_as(&pool, 1, "attached", 4096).await;
    assert_eq!(
        filled(&chat, 1).await,
        Ok(expected),
        "a fresh value is served"
    );
    assert_eq!(filled(&chat, 2).await, Ok(1 << 30));
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_fill_reads_only_the_covering_index(pool: PgPool) {
    let mut conn = pool.acquire().await.expect("a connection");
    for off in ["SET enable_seqscan = off", "SET enable_bitmapscan = off"] {
        sqlx::query(off)
            .execute(&mut *conn)
            .await
            .expect("a planner switch");
    }
    let plan: Vec<String> = sqlx::query_scalar(&format!(
        "EXPLAIN (COSTS OFF) {}",
        include_str!("../../../../queries/chat/media/store_fill.sql")
    ))
    .bind(1_i16)
    .fetch_all(&mut *conn)
    .await
    .expect("a plan");
    let plan = plan.join("\n");
    assert!(plan.contains("Index Only Scan using media_fill"), "{plan}");
}
