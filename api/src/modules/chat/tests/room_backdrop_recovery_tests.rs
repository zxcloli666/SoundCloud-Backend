use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::backdrop_support::{Bench, image, put, someone, steal};
use super::report_support::{file, report, stage};
use crate::modules::chat::media::backdrop::{self, Attached, lock::Held};
use crate::modules::chat::rt_keys;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_busy_lock_delays_the_crowd_clear_until_the_next_report(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, first, second, third) = (someone(), someone(), someone(), someone());
    let stage = stage(&bench, &host, true, &[&first, &second, &third]).await?;
    let (conversation, media) = (stage.conversation, stage.media);
    for user in [&first, &second] {
        assert_eq!(
            file(&bench, user, report(conversation, media)).await?.0,
            StatusCode::OK
        );
    }
    steal(&bench, conversation, 10_000).await?;
    let (status, filed) = file(&bench, &third, report(conversation, media)).await?;
    assert_eq!(
        (status, &filed["duplicate"]),
        (StatusCode::OK, &json!(false))
    );
    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.image_backdrop(), Some(media), "the lock was busy");

    let mut conn = bench.chat.redis.get().await?;
    let _: i64 = redis::cmd("DEL")
        .arg(rt_keys::backdrop_lock(conversation))
        .query_async(&mut conn)
        .await?;
    drop(conn);
    let (status, again) = file(&bench, &third, report(conversation, media)).await?;
    assert_eq!(
        (status, &again["duplicate"]),
        (StatusCode::OK, &json!(true))
    );
    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.backdrop, None);
    assert!(room.backdrop_cooldown_until.is_some());
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "held");
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_new_image_is_reportable_once_its_meta_is_posted(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    let stage = stage(&bench, &host, false, &[&guest]).await?;
    let conversation = stage.conversation;
    let fresh = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &stage.code, image(fresh)).await?.0,
        StatusCode::OK
    );
    assert_eq!(bench.meta_of(conversation).await?, None);
    assert_eq!(
        file(&bench, &guest, report(conversation, fresh)).await?,
        (
            StatusCode::CONFLICT,
            json!({"error": "chat_media_missing", "ids": [fresh]})
        )
    );
    bench.mark_meta(conversation).await?;
    let (status, filed) = file(&bench, &guest, report(conversation, fresh)).await?;
    assert_eq!(
        (status, &filed["duplicate"]),
        (StatusCode::OK, &json!(false))
    );
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_failed_room_write_gives_the_upload_and_the_meta_back(pg: PgPool) -> anyhow::Result<()> {
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
    let (body, epoch): (Vec<u8>, i32) = sqlx::query_as(
        "UPDATE chat.conversations AS c SET meta_ct = NULL, meta_epoch = NULL FROM chat.conversations AS old WHERE c.id = $1 AND old.id = c.id RETURNING old.meta_ct, old.meta_epoch",
    )
    .bind(conversation)
    .fetch_one(&bench.pg)
    .await?;
    let media = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let usage = bench.usage_of(&host).await?;
    sqlx::query("INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, 0)")
        .bind(media)
        .bind(conversation)
        .execute(&bench.pg)
        .await?;
    sqlx::query("UPDATE chat.media SET status = 'attached', delete_after = NULL WHERE id = $1")
        .bind(media)
        .execute(&bench.pg)
        .await?;
    let attached = Attached {
        media,
        before: Some(shown),
        flipped: true,
        meta: Some((body.clone(), epoch)),
    };
    let held = Held::take(&bench.chat, conversation)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(
        backdrop::tidy(&bench.chat, &held, conversation, Some(&attached), None).await,
        Ok(true)
    );
    held.release(&bench.chat).await;
    assert_eq!(bench.status_of(media).await?, "uploaded");
    assert_eq!(bench.refs_of(conversation).await?, [(shown, 0)]);
    assert_eq!(bench.usage_of(&host).await?, usage);
    assert_eq!(bench.meta_of(conversation).await?, Some(body));

    let (status, view) = put(&bench, &host, &code, image(media)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(bench.refs_of(conversation).await?, [(media, 0)]);
    assert_eq!(bench.status_of(shown).await?, "deleting");
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_old_hosts_image_keeps_its_ref_until_the_room_ends(pg: PgPool) -> anyhow::Result<()> {
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
    bench.chat.rooms.leave(&code, &first).await?;
    let room = bench.chat.rooms.room(&code).await?;
    assert_eq!(
        (room.host_id.as_str(), room.backdrop.clone()),
        (next.as_str(), None)
    );

    let (status, view) = put(
        &bench,
        &next,
        &code,
        json!({"backdrop": {"kind": "preset", "id": "dunes"}}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    let mine = bench
        .upload(&next, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &next, &code, image(mine)).await?.0,
        StatusCode::OK
    );
    let (status, view) = put(&bench, &next, &code, json!({"backdrop": null})).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(bench.refs_of(conversation).await?, [(old, 0)]);
    assert_eq!(bench.status_of(old).await?, "attached");
    assert_eq!(bench.status_of(mine).await?, "deleting");
    assert_eq!(
        backdrop::clear(&bench.chat, conversation, old, None).await,
        Ok(false)
    );
    assert!(
        bench.refs_of(conversation).await?.is_empty(),
        "an admin clear releases it"
    );
    assert_eq!(bench.status_of(old).await?, "deleting");
    bench.chat.rooms.end(&code).await?;
    Ok(())
}
