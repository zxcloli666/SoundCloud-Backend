use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{Bench, image, preset, put, someone, steal};
use crate::error::AppError;
use crate::modules::chat::media::backdrop::{self, Settle, lock::Held};
use crate::modules::chat::rt_keys;
use crate::modules::rooms::model::SettingsPatch;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn two_hosts_tabs_racing_leave_one_backdrop_and_one_ref(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let uri = format!("/chat/rooms/{code}/settings");
    for _ in 0..3 {
        bench.forget_limits(&code).await?;
        let first = bench
            .upload(&host, conversation, "backdrop", "uploaded")
            .await?;
        let second = bench
            .upload(&host, conversation, "backdrop", "uploaded")
            .await?;
        let (left, right) = tokio::join!(
            bench.ask(&host, "PUT", &uri, Some(image(first))),
            bench.ask(&host, "PUT", &uri, Some(image(second))),
        );
        assert_eq!((left?.0, right?.0), (StatusCode::OK, StatusCode::OK));
        let shown = bench
            .chat
            .rooms
            .room(&code)
            .await?
            .image_backdrop()
            .expect("an image backdrop");
        assert_eq!(bench.refs_of(conversation).await?, [(shown, 0)]);
        let lost = if shown == first { second } else { first };
        assert_eq!(bench.status_of(shown).await?, "attached");
        assert_eq!(bench.status_of(lost).await?, "deleting");
    }
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_ref_left_by_a_crash_goes_with_the_next_change(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let stray = bench
        .upload(&host, conversation, "backdrop", "attached")
        .await?;
    sqlx::query("INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, 0)")
        .bind(stray)
        .bind(conversation)
        .execute(&bench.pg)
        .await?;
    let before = bench.usage_of(&host).await?;
    let (status, view) = bench
        .ask(
            &host,
            "PUT",
            &format!("/chat/rooms/{code}/settings"),
            Some(json!({"backdrop": {"kind": "preset", "id": "paper"}})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(stray).await?, "deleting");
    assert_eq!(
        bench.usage_of(&host).await?,
        (before.0 - 4096, before.1 - 1)
    );
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_lock_left_by_a_dead_process_runs_out_and_the_change_goes_through(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let media = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let started = tokio::time::Instant::now();
    steal(&bench, conversation, 400).await?;
    let (status, view) = bench
        .ask(
            &host,
            "PUT",
            &format!("/chat/rooms/{code}/settings"),
            Some(image(media)),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(started.elapsed() >= Duration::from_millis(350));
    assert_eq!(bench.refs_of(conversation).await?, [(media, 0)]);
    let mut conn = bench.chat.redis.get().await?;
    let left: Option<String> = redis::cmd("GET")
        .arg(rt_keys::backdrop_lock(conversation))
        .query_async(&mut conn)
        .await?;
    assert_eq!(left, None, "the lock is released after the change");
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_holder_whose_lock_was_taken_over_changes_nothing(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let shown = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &code, image(shown)).await?.0,
        StatusCode::OK
    );

    let held = Held::take(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let stale_fence: Option<Uuid> =
        sqlx::query_scalar("SELECT backdrop_fence FROM chat.conversations WHERE id = $1")
            .bind(conversation)
            .fetch_one(&bench.pg)
            .await?;
    steal(&bench, conversation, 10_000).await?;
    let patch: SettingsPatch = serde_json::from_value(preset("vinyl"))?;
    let refused = bench
        .chat
        .rooms
        .reconfigure_guarded(&code, &host, &patch, None, Some(&held.guard()))
        .await;
    assert!(
        matches!(refused, Err(AppError::Conflict(_))),
        "a stale holder never writes the room: {refused:?}"
    );
    assert_eq!(
        bench.chat.rooms.room(&code).await?.image_backdrop(),
        Some(shown)
    );

    let mut conn = bench.chat.redis.get().await?;
    let _: i64 = redis::cmd("DEL")
        .arg(rt_keys::backdrop_lock(conversation))
        .query_async(&mut conn)
        .await?;
    drop(conn);
    let newer = Held::take(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let fresh = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let (fence, attached) = backdrop::attach(&bench.chat, conversation, &host, fresh)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let wanted: SettingsPatch = serde_json::from_value(image(fresh))?;
    let (_, replaced) = bench
        .chat
        .rooms
        .reconfigure_guarded(&code, &host, &wanted, None, Some(&newer.guard()))
        .await?;
    assert_eq!(replaced, Some(shown));

    let stale = Settle {
        fence: stale_fence,
        released: Some(fresh),
        ..Settle::default()
    };
    assert_eq!(
        backdrop::tidy(&bench.chat, conversation, stale).await,
        Ok(())
    );
    let mut refs = vec![(shown, 0), (fresh, 0)];
    refs.sort();
    assert_eq!(bench.refs_of(conversation).await?, refs);
    assert_eq!(bench.status_of(fresh).await?, "attached");

    let settle = Settle {
        fence: Some(fence),
        attached: Some(attached),
        written: true,
        replaced,
        released: None,
    };
    assert_eq!(
        backdrop::tidy(&bench.chat, conversation, settle).await,
        Ok(())
    );
    newer.release(&bench.chat).await;
    assert_eq!(bench.refs_of(conversation).await?, [(fresh, 0)]);
    assert_eq!(bench.status_of(shown).await?, "deleting");
    assert_eq!(bench.status_of(fresh).await?, "attached");
    held.release(&bench.chat).await;
    bench.chat.rooms.end(&code).await?;
    Ok(())
}
