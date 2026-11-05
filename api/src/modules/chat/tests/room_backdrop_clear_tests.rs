use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{BLOCK, Bench, lock_runs_out, preset, put, someone, steal};
use super::report_support::{file, report, stage};
use crate::modules::chat::media::backdrop::{self, lock::Held};
use crate::modules::chat::media::support::{cancel_a_lock_wait, until_blocked};

async fn signed(
    bench: &Bench,
    user: &str,
    conversation: Uuid,
    media: Uuid,
) -> anyhow::Result<String> {
    let (status, body) = bench
        .ask(
            user,
            "POST",
            "/chat/media/urls",
            Some(json!({"conversation_id": conversation, "ids": [media]})),
        )
        .await?;
    anyhow::ensure!(status == StatusCode::OK, "{body}");
    Ok(body["items"][0]["state"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_cleared_image_stops_signing_even_when_both_tidies_fail(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::signing(pg)?;
    let (host, guest) = (someone(), someone());
    let stage = stage(&bench, &host, true, &[&guest]).await?;
    let (conversation, shown) = (stage.conversation, stage.media);
    let leftover = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let held = Held::take(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    backdrop::attach(&bench.chat, conversation, &host, leftover)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    held.release(&bench.chat).await;
    assert_eq!(
        file(&bench, &guest, report(conversation, shown)).await?.0,
        StatusCode::OK
    );
    assert_eq!(signed(&bench, &guest, conversation, shown).await?, "ok");
    let (bytes, files) = bench.usage_of(&host).await?;

    let mut blocker = bench.pg.begin().await?;
    sqlx::query("SELECT id FROM chat.media WHERE id = $1 FOR UPDATE")
        .bind(leftover)
        .execute(&mut *blocker)
        .await?;
    let clearing = {
        let chat = bench.chat.clone();
        tokio::spawn(async move { backdrop::clear(&chat, conversation, shown, None).await })
    };
    let first = cancel_a_lock_wait(&bench.pg, None).await?;
    cancel_a_lock_wait(&bench.pg, Some(first)).await?;
    assert_eq!(clearing.await?, Ok(true));
    blocker.rollback().await?;

    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.image_backdrop(), None);
    assert_eq!(bench.refs_of(conversation).await?, [(leftover, 0)]);
    assert_eq!(bench.status_of(shown).await?, "held");
    assert_eq!(bench.usage_of(&host).await?, (bytes - BLOCK, files - 1));
    for user in [&guest, &host] {
        assert_eq!(signed(&bench, user, conversation, shown).await?, "deleted");
    }

    let (status, view) = put(&bench, &host, &stage.code, preset("dunes")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(leftover).await?, "deleting");
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_clear_whose_room_write_fails_signs_nothing_and_the_next_clear_ends_it(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::signing(pg)?;
    let (host, guest) = (someone(), someone());
    let stage = stage(&bench, &host, true, &[&guest]).await?;
    let (conversation, shown) = (stage.conversation, stage.media);
    assert_eq!(
        file(&bench, &guest, report(conversation, shown)).await?.0,
        StatusCode::OK
    );

    let mut blocker = bench.pg.begin().await?;
    sqlx::query("SELECT id FROM chat.conversations WHERE id = $1 FOR UPDATE")
        .bind(conversation)
        .execute(&mut *blocker)
        .await?;
    let clearing = {
        let chat = bench.chat.clone();
        tokio::spawn(async move { backdrop::clear(&chat, conversation, shown, None).await })
    };
    until_blocked(&bench.pg).await;
    steal(&bench, conversation, 10_000).await?;
    blocker.rollback().await?;
    assert!(clearing.await?.is_err());

    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.image_backdrop(), Some(shown));
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(shown).await?, "held");
    for user in [&guest, &host] {
        assert_eq!(signed(&bench, user, conversation, shown).await?, "deleted");
    }

    lock_runs_out(&bench, conversation).await?;
    assert_eq!(
        backdrop::clear(&bench.chat, conversation, shown, None).await,
        Ok(true)
    );
    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.image_backdrop(), None);
    assert_eq!(bench.status_of(shown).await?, "held");
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}
