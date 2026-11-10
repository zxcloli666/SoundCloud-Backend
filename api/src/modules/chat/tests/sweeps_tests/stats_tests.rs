use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, member, messages, ok, user};
use crate::modules::chat::sweeps::stats;
use crate::modules::chat::test_support::chat_state;

async fn image(pg: &PgPool, owner: &str, origin: Uuid, bytes: i64) -> anyhow::Result<()> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, parent_id, bytes, status)
         VALUES ($1, 1, 'i/' || $1, $2, $3, 'image', NULL, $4, 'attached'),
                (gen_random_uuid(), 1, 't/' || $1, $2, $3, 'thumb', $1, 512, 'attached')",
    )
    .bind(id)
    .bind(owner)
    .bind(origin)
    .bind(bytes)
    .execute(pg)
    .await?;
    Ok(())
}

async fn device(pg: &PgPool, owner: &str) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO chat.users (user_id) VALUES ($1)")
        .bind(owner)
        .execute(pg)
        .await?;
    sqlx::query(
        "INSERT INTO chat.devices
            (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier)
         VALUES (gen_random_uuid(), $1, sha256(uuid_send(gen_random_uuid())),
                 '\\x01'::bytea, '\\x02'::bytea, 'macos', 'MacBook', '8.7.0', 'direct')",
    )
    .bind(owner)
    .execute(pg)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_volume_gauges_count_what_is_stored_and_drop_back_to_zero(
    pg: PgPool,
) -> anyhow::Result<()> {
    crate::metrics::init();
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(pg.clone(), redis, true);
    let owner = user();
    let dm = conversation(&pg, "direct", &owner).await?;
    member(&pg, dm, &owner, "active").await?;
    messages(&pg, dm, 1..=3, None).await?;
    conversation(&pg, "group", &owner).await?;
    image(&pg, &owner, dm, 4096).await?;
    device(&pg, &owner).await?;

    ok(stats::run_once(&chat).await)?;

    let body = crate::metrics::render_for_tests().unwrap_or_default();
    for line in [
        "chat_conversations{kind=\"direct\"} 1",
        "chat_conversations{kind=\"group\"} 1",
        "chat_conversations{kind=\"room\"} 0",
        "chat_conversations_active{kind=\"direct\",window=\"1d\"} 1",
        "chat_members{kind=\"direct\"} 0",
        "chat_media_objects{kind=\"image\",status=\"attached\"} 1",
        "chat_media_bytes{kind=\"image\",status=\"attached\"} 4096",
        "chat_media_bytes{kind=\"thumb\",status=\"attached\"} 512",
        "chat_media_objects{kind=\"backdrop\",status=\"held\"} 0",
        "chat_users 1",
        "chat_users_new{window=\"1d\"} 1",
        "chat_users_active{window=\"7d\"} 1",
        "chat_devices_active 1",
        "chat_reports_open 0",
        "chat_table_bytes{table=\"messages\"}",
        "chat_table_rows{table=\"media\"}",
    ] {
        assert!(body.contains(line), "{line} is missing from\n{body}");
    }

    sqlx::query("DELETE FROM chat.media").execute(&pg).await?;
    ok(stats::run_once(&chat).await)?;

    let body = crate::metrics::render_for_tests().unwrap_or_default();
    for line in [
        "chat_media_objects{kind=\"image\",status=\"attached\"} 0",
        "chat_media_bytes{kind=\"thumb\",status=\"attached\"} 0",
    ] {
        assert!(body.contains(line), "{line} is missing from\n{body}");
    }
    Ok(())
}
