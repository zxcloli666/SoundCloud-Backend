use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::backdrop_support::{Bench, image, preset, put, someone, steal};
use crate::modules::chat::media::backdrop::{self, Settle, lock::Held};
use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{owner_delete, pass_window};
use crate::modules::chat::sweeps::media::withdrawn;
use crate::modules::rooms::model::SettingsPatch;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_handoff_before_the_tidy_still_frees_the_replaced_image(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (first, next) = (someone(), someone());
    bench.star(&first).await?;
    bench.star(&next).await?;
    let code = bench.room(&first, false, &[&next]).await?;
    let conversation = bench.open_chat(&code, &first).await?;
    let old = bench
        .upload(&first, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &first, &code, image(old)).await?.0,
        StatusCode::OK
    );
    let fresh = bench
        .upload(&first, conversation, "backdrop", "uploaded")
        .await?;
    let held = Held::take(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let (fence, attached) = backdrop::attach(&bench.chat, conversation, &first, fresh)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let patch: SettingsPatch = serde_json::from_value(image(fresh))?;
    let (_, replaced) = bench
        .chat
        .rooms
        .reconfigure_guarded(&code, &first, &patch, None, Some(&held.guard()))
        .await?;
    assert_eq!(replaced, Some(old));
    bench.chat.rooms.leave(&code, &first).await?;
    let room = bench.chat.rooms.room(&code).await?;
    assert_eq!(
        (room.host_id.as_str(), room.backdrop),
        (next.as_str(), None)
    );

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
    held.release(&bench.chat).await;
    assert_eq!(bench.refs_of(conversation).await?, [(fresh, 0)]);
    assert_eq!(bench.status_of(old).await?, "deleting");
    assert_eq!(bench.status_of(fresh).await?, "attached");
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_backdrop_deleted_for_good_leaves_the_room(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone())?;
    let (host, guest) = (someone(), someone());
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[&guest]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let media = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &code, image(media)).await?.0,
        StatusCode::OK
    );
    assert!(owner_delete(&pg, media, 60).await?);
    let swept = withdrawn::run(&bench.chat, &mut Budget::new(5))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(swept, 0, "the restore window still runs");
    assert_eq!(
        bench.chat.rooms.room(&code).await?.image_backdrop(),
        Some(media)
    );

    pass_window(&pg, media).await?;
    let swept = withdrawn::run(&bench.chat, &mut Budget::new(5))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(swept, 1);
    let view = bench.chat.rooms.view(&code, &guest).await?;
    assert_eq!(view.backdrop, None);
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "deleting");
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn only_applied_changes_use_the_budget_of_one_room(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, true, &[]).await?;
    let budget = bench.chat.rooms.room(&code).await?.id;
    for id in ["aurora", "vinyl"] {
        let (status, view) = put(&bench, &host, &code, preset(id)).await?;
        assert_eq!(status, StatusCode::OK, "{view}");
    }
    let code = bench.chat.rooms.set_public(&code, &host, false).await?.code;
    let conversation = bench.open_chat(&code, &host).await?;
    assert_eq!(bench.chat.rooms.room(&code).await?.id, budget);

    steal(&bench, conversation, 10_000).await?;
    let (status, _) = put(&bench, &host, &code, preset("dunes")).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "the lock was busy");
    let mut conn = bench.chat.redis.get().await?;
    let _: i64 = redis::cmd("DEL")
        .arg(rt_keys::backdrop_lock(conversation))
        .query_async(&mut conn)
        .await?;
    drop(conn);

    for id in ["dunes", "satin", "paper", "candy-sky"] {
        let (status, view) = put(&bench, &host, &code, preset(id)).await?;
        assert_eq!(status, StatusCode::OK, "{view}");
    }
    assert_eq!(
        put(&bench, &host, &code, preset("watercolor")).await?,
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "chat_request_limit"})
        )
    );
    bench.chat.rooms.end(&code).await?;
    Ok(())
}
