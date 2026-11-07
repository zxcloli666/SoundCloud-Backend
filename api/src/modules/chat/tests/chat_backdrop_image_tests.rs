use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{BLOCK, Bench, chosen, image, missing, preset, refused, report, someone};
use crate::modules::chat::media::backdrop;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{owner_delete, pass_window};
use crate::modules::chat::sweeps::media::withdrawn;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_own_upload_from_this_chat_becomes_the_image_and_a_replaced_one_is_dropped(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg);
    let (first, second) = (someone(), someone());
    bench.star(&first).await?;
    bench.star(&second).await?;
    let dm = bench.dm(&first, &second).await;

    let foreign = bench.upload(&second, dm, "backdrop", "uploaded").await?;
    let elsewhere = bench
        .upload(&first, Uuid::now_v7(), "backdrop", "uploaded")
        .await?;
    let pending = bench.upload(&first, dm, "backdrop", "reserved").await?;
    let picture = bench.upload(&first, dm, "image", "uploaded").await?;
    for wrong in [foreign, elsewhere, pending, picture, Uuid::new_v4()] {
        assert_eq!(bench.put(&first, dm, image(wrong)).await?, missing(wrong));
    }
    assert!(bench.refs_of(dm).await?.is_empty());

    let mine = bench.upload(&first, dm, "backdrop", "uploaded").await?;
    let (status, view) = bench.put(&first, dm, image(mine)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], chosen(image(mine), &first));
    assert_eq!(bench.refs_of(dm).await?, [(mine, 0)]);
    assert_eq!(bench.status_of(mine).await?, "attached");
    assert_eq!(
        bench.show(&second, dm).await?["backdrop"],
        chosen(image(mine), &first)
    );

    let usage = bench.usage_of(&first).await?;
    let next = bench.upload(&first, dm, "backdrop", "uploaded").await?;
    let (status, _) = bench.put(&first, dm, image(next)).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bench.refs_of(dm).await?, [(next, 0)]);
    assert_eq!(bench.status_of(mine).await?, "deleting");
    assert_eq!(bench.status_of(next).await?, "attached");
    assert_eq!(bench.usage_of(&first).await?, usage);

    let (status, view) = bench.put(&second, dm, image(foreign)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], chosen(image(foreign), &second));
    assert_eq!(bench.refs_of(dm).await?, [(foreign, 0)]);
    assert_eq!(bench.status_of(next).await?, "deleting");

    let theirs = bench.usage_of(&second).await?;
    let (status, view) = bench.put(&first, dm, preset("satin")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(bench.refs_of(dm).await?.is_empty());
    assert_eq!(bench.status_of(foreign).await?, "deleting");
    assert_eq!(
        bench.usage_of(&second).await?,
        (theirs.0 - BLOCK, theirs.1 - 1)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_meta_message_carries_the_live_backdrop_and_keeps_the_group_avatar(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg);
    let (first, second) = (someone(), someone());
    bench.star(&first).await?;
    bench.star(&second).await?;
    let dm = bench.dm(&first, &second).await;
    let media = bench.upload(&first, dm, "backdrop", "uploaded").await?;
    assert_eq!(bench.put(&first, dm, image(media)).await?.0, StatusCode::OK);

    assert_eq!(
        bench.send_meta(&first, dm, &[]).await?,
        refused(StatusCode::BAD_REQUEST, "chat_invalid")
    );
    let other = bench.upload(&first, dm, "backdrop", "uploaded").await?;
    assert_eq!(bench.send_meta(&first, dm, &[other]).await?, missing(other));
    let (status, sent) = bench.send_meta(&first, dm, &[media]).await?;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert!(bench.meta_of(dm).await?.is_some());
    assert!(bench.show(&second, dm).await?["meta_ct"].is_string());
    assert_eq!(bench.refs_of(dm).await?, [(media, 0)]);

    let group = bench.group(&first, &[], &[&second]).await?;
    let avatar = bench.upload(&first, group, "avatar", "uploaded").await?;
    let look = bench.upload(&first, group, "backdrop", "uploaded").await?;
    assert_eq!(
        bench.put(&first, group, image(look)).await?.0,
        StatusCode::OK
    );
    let (status, sent) = bench.send_meta(&first, group, &[avatar, look]).await?;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let mut both = vec![(avatar, 0), (look, 0)];
    both.sort();
    assert_eq!(bench.refs_of(group).await?, both);
    let (status, sent) = bench.send_meta(&first, group, &[avatar]).await?;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(
        bench.refs_of(group).await?,
        both,
        "an avatar meta keeps the backdrop"
    );
    let (status, sent) = bench.send_meta(&first, group, &[look]).await?;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(bench.refs_of(group).await?, [(look, 0)]);
    assert_eq!(bench.status_of(avatar).await?, "deleting");
    assert_eq!(
        bench.send_meta(&first, group, &[look, other]).await?,
        missing(other)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_image_deleted_for_good_or_cleared_by_an_admin_leaves_the_chat(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone());
    let user = someone();
    bench.star(&user).await?;
    let saved = bench.saved(&user).await?;
    let media = bench.upload(&user, saved, "backdrop", "uploaded").await?;
    assert_eq!(
        bench.put(&user, saved, image(media)).await?.0,
        StatusCode::OK
    );

    assert!(owner_delete(&pg, media, 60).await?);
    let swept = withdrawn::run(&bench.chat, &mut Budget::new(5))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(swept, 0, "the restore window still runs");
    assert_eq!(bench.stored(saved).await?, chosen(image(media), &user));
    pass_window(&pg, media).await?;
    let swept = withdrawn::run(&bench.chat, &mut Budget::new(5))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(swept, 1);
    assert_eq!(bench.stored(saved).await?, json!(null));
    assert_eq!(bench.show(&user, saved).await?["backdrop"], json!(null));
    assert!(bench.refs_of(saved).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "deleting");

    let next = bench.upload(&user, saved, "backdrop", "uploaded").await?;
    assert_eq!(
        bench.put(&user, saved, image(next)).await?.0,
        StatusCode::OK
    );
    let cleared = backdrop::clear(&bench.chat, saved, next, None)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert!(cleared);
    assert_eq!(bench.stored(saved).await?, json!(null));
    assert!(bench.refs_of(saved).await?.is_empty());
    assert_eq!(bench.status_of(next).await?, "deleting");
    let again = backdrop::clear(&bench.chat, saved, next, None)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert!(!again);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn members_report_the_image_and_a_group_never_clears_it_by_count(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg);
    let (owner, first, second, third, stranger) =
        (someone(), someone(), someone(), someone(), someone());
    bench.star(&owner).await?;
    let group = bench.group(&owner, &[], &[&first, &second, &third]).await?;
    let media = bench.upload(&owner, group, "backdrop", "uploaded").await?;
    assert_eq!(
        bench.put(&owner, group, image(media)).await?.0,
        StatusCode::OK
    );
    assert_eq!(
        bench.file(&first, report(group, media)).await?.1["error"],
        json!("chat_media_missing"),
        "a report waits for the meta that carries the key"
    );
    assert_eq!(
        bench.send_meta(&owner, group, &[media]).await?.0,
        StatusCode::OK
    );

    let (status, filed) = bench.file(&first, report(group, media)).await?;
    assert_eq!(status, StatusCode::OK, "{filed}");
    assert_eq!(
        (&filed["state"], &filed["duplicate"]),
        (&json!("open"), &json!(false))
    );
    let (status, again) = bench.file(&first, report(group, media)).await?;
    assert_eq!(
        (status, &again["duplicate"]),
        (StatusCode::OK, &json!(true))
    );
    assert_eq!(bench.holds_of(media).await?, 1);
    assert_eq!(
        bench.file(&owner, report(group, media)).await?,
        refused(StatusCode::BAD_REQUEST, "chat_invalid")
    );
    assert_eq!(
        bench.file(&stranger, report(group, media)).await?,
        refused(StatusCode::FORBIDDEN, "chat_not_member")
    );
    let other = Uuid::new_v4();
    assert_eq!(
        bench.file(&second, report(group, other)).await?,
        missing(other)
    );
    for reporter in [&second, &third] {
        assert_eq!(
            bench.file(reporter, report(group, media)).await?.0,
            StatusCode::OK
        );
    }
    assert_eq!(bench.stored(group).await?, chosen(image(media), &owner));
    assert_eq!(bench.refs_of(group).await?, [(media, 0)]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_backdrop_reserve_follows_the_same_roles_and_needs_star(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg);
    let (owner, member, free) = (someone(), someone(), someone());
    bench.star(&owner).await?;
    bench.star(&member).await?;
    let dm = bench.dm(&owner, &free).await;
    let group = bench.group(&owner, &[], &[&member]).await?;
    let body = |conversation: Uuid| {
        json!({
            "upload_id": Uuid::new_v4(), "conversation_id": conversation,
            "kind": "backdrop", "bytes": 614_400
        })
    };
    let (status, reserved) = bench
        .ask(&owner, "POST", "/chat/media/reserve", Some(body(dm)))
        .await?;
    assert_eq!(status, StatusCode::OK, "{reserved}");
    assert!(reserved["put"]["url"].is_string(), "{reserved}");
    assert_eq!(
        bench
            .ask(&free, "POST", "/chat/media/reserve", Some(body(dm)))
            .await?,
        refused(StatusCode::FORBIDDEN, "chat_media_star_required")
    );
    assert_eq!(
        bench
            .ask(&member, "POST", "/chat/media/reserve", Some(body(group)))
            .await?,
        refused(StatusCode::FORBIDDEN, "chat_not_member")
    );
    assert_eq!(
        bench
            .ask(&member, "POST", "/chat/media/reserve", Some(body(dm)))
            .await?,
        refused(StatusCode::FORBIDDEN, "chat_not_member")
    );
    Ok(())
}
