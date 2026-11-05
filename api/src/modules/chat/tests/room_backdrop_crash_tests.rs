use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{Bench, image, lock_runs_out, preset, put, someone, steal};
use super::report_support::{file, report, stage};
use crate::modules::chat::media::backdrop::{self, Settle, lock::Held};
use crate::modules::rooms::model::SettingsPatch;

async fn take(bench: &Bench, conversation: Uuid) -> anyhow::Result<Held> {
    Held::take(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

fn sorted(mut refs: Vec<(Uuid, i64)>) -> Vec<(Uuid, i64)> {
    refs.sort();
    refs
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_crash_before_the_room_write_leaves_the_old_backdrop_whole(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    let stage = stage(&bench, &host, true, &[&guest]).await?;
    let (conversation, shown) = (stage.conversation, stage.media);
    let fresh = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let held = take(&bench, conversation).await?;
    backdrop::attach(&bench.chat, conversation, &host, fresh)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    drop(held);

    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.image_backdrop(), Some(shown));
    assert_eq!(bench.meta_of(conversation).await?, Some(stage.meta.clone()));
    let (status, filed) = file(&bench, &guest, report(conversation, shown)).await?;
    assert_eq!(
        (status, &filed["duplicate"]),
        (StatusCode::OK, &json!(false)),
        "the shown image stays reportable"
    );
    assert_eq!(
        bench.refs_of(conversation).await?,
        sorted(vec![(shown, 0), (fresh, 0)])
    );

    lock_runs_out(&bench, conversation).await?;
    let (status, view) = put(&bench, &host, &stage.code, preset("dunes")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(fresh).await?, "deleting");
    assert_eq!(bench.status_of(shown).await?, "held");
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_crash_after_the_room_write_goes_with_the_next_change(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let old = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &code, image(old)).await?.0,
        StatusCode::OK
    );
    bench.mark_meta(conversation).await?;
    let fresh = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let held = take(&bench, conversation).await?;
    backdrop::attach(&bench.chat, conversation, &host, fresh)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let patch: SettingsPatch = serde_json::from_value(image(fresh))?;
    bench
        .chat
        .rooms
        .reconfigure_guarded(&code, &host, &patch, None, Some(&held.guard()))
        .await?;
    drop(held);
    assert_eq!(
        bench.chat.rooms.room(&code).await?.image_backdrop(),
        Some(fresh)
    );
    assert_eq!(bench.status_of(fresh).await?, "attached");

    lock_runs_out(&bench, conversation).await?;
    let last = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let (status, view) = put(&bench, &host, &code, image(last)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(bench.refs_of(conversation).await?, [(last, 0)]);
    assert_eq!(bench.status_of(old).await?, "deleting");
    assert_eq!(bench.status_of(fresh).await?, "deleting");
    assert_eq!(bench.meta_of(conversation).await?, None);
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_failed_room_write_gives_the_upload_back_and_keeps_the_meta(
    pg: PgPool,
) -> anyhow::Result<()> {
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
    bench.mark_meta(conversation).await?;
    let meta = bench.meta_of(conversation).await?;
    let fresh = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let usage = bench.usage_of(&host).await?;
    let held = take(&bench, conversation).await?;
    let (fence, attached) = backdrop::attach(&bench.chat, conversation, &host, fresh)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    steal(&bench, conversation, 10_000).await?;
    let patch: SettingsPatch = serde_json::from_value(image(fresh))?;
    let refused = bench
        .chat
        .rooms
        .reconfigure_guarded(&code, &host, &patch, None, Some(&held.guard()))
        .await;
    assert!(refused.is_err());
    let settle = Settle {
        fence: Some(fence),
        attached: Some(attached),
        ..Settle::default()
    };
    assert_eq!(
        backdrop::tidy(&bench.chat, conversation, settle).await,
        Ok(())
    );
    assert_eq!(bench.status_of(fresh).await?, "uploaded");
    assert_eq!(bench.refs_of(conversation).await?, [(shown, 0)]);
    assert_eq!(bench.usage_of(&host).await?, usage);
    assert_eq!(bench.meta_of(conversation).await?, meta);

    lock_runs_out(&bench, conversation).await?;
    let (status, view) = put(&bench, &host, &code, image(fresh)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(bench.refs_of(conversation).await?, [(fresh, 0)]);
    assert_eq!(bench.status_of(shown).await?, "deleting");
    assert_eq!(bench.meta_of(conversation).await?, None);
    bench.chat.rooms.end(&code).await?;
    Ok(())
}
