use sqlx::PgPool;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::limits::QUOTA_BYTES;
use crate::modules::chat::media::support::{
    IMAGE, Rig, THUMB, bench, bench_with, call, image_body, reserve, star, user,
};
use crate::modules::chat::media::view;

const TOTAL: i64 = (IMAGE + THUMB) as i64;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_config_media_block_follows_the_switch_the_cap_and_star(pool: PgPool) {
    let owner = user();
    star(&pool, &owner).await;
    let open = view::config(&bench(&pool), &owner).await;
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
    let closed = view::config(&bench_with(&pool, &full), &owner).await;
    assert!(!closed.uploads && closed.star);
}

async fn rename(pool: &PgPool, from: &str, to: &str) {
    sqlx::query(&format!("ALTER TABLE {from} RENAME TO {to}"))
        .execute(pool)
        .await
        .expect("a rename");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_failed_media_lookup_serves_chat_config_without_uploads_and_star(pool: PgPool) {
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    reserve(&bench(&pool), &owner, image_body(group)).await;
    for (table, schema, gone) in [
        ("subscriptions", "public", "subscriptions_gone"),
        ("usage", "chat", "usage_gone"),
        ("media", "chat", "media_gone"),
    ] {
        let chat = bench(&pool);
        let open = view::config(&chat, &owner).await;
        assert!(open.uploads && open.star && open.used > 0, "{table}");
        rename(&pool, &format!("{schema}.{table}"), gone).await;
        let chat = bench(&pool);
        let degraded = view::config(&chat, &owner).await;
        assert!(!degraded.uploads && !degraded.star, "{table}");
        assert_eq!((degraded.quota, degraded.used), (QUOTA_BYTES, 0), "{table}");
        assert_eq!(degraded.limits, view::limits());
        let config = call(&chat, &owner, "GET", "/chat/config", None).await;
        assert_eq!(config.0, 200, "{table}: {config:?}");
        assert_eq!(config.1["media"]["uploads"], false, "{table}");
        assert_eq!(config.1["media"]["star"], false, "{table}");
        assert_eq!(config.1["user_id"], owner.as_str(), "{table}");
        rename(&pool, &format!("{schema}.{gone}"), table).await;
    }
}
