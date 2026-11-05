use sqlx::PgPool;

use super::support::{conversation, member, state, user};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    Usage, age, attach, handle, hide, ok, owner_delete, pass_window, redact, refs, report, reserve,
    snapshot, status, statuses, upload, usage,
};
use crate::modules::chat::sweeps::media::{expire, holds, trim, withdrawn};

fn deleting() -> Vec<Option<String>> {
    vec![Some("deleting".to_owned()); 2]
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_stale_reservation_and_an_unsent_upload_expire_and_give_the_quota_back(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let origin = conversation(&pg, "group", &owner).await?;
    let stale = reserve(&pg, &owner, origin, 1, true).await?;
    let unsent = upload(&pg, &owner, origin, 1).await?;
    let fresh = upload(&pg, &owner, origin, 1).await?;
    age(&pg, stale.id).await?;
    age(&pg, unsent.id).await?;

    assert_eq!(ok(expire::run(&chat, &mut Budget::new(5)).await)?, 2);

    assert_eq!(statuses(&pg, &stale).await?, deleting());
    assert_eq!(statuses(&pg, &unsent).await?, deleting());
    assert_eq!(status(&pg, fresh.id).await?.as_deref(), Some("uploaded"));
    assert_eq!(
        usage(&pg, &owner).await?,
        Usage {
            bytes: fresh.total(),
            reserved: 0,
            files: 1
        }
    );
    let before = snapshot(&pg).await?;
    assert_eq!(ok(expire::run(&chat, &mut Budget::new(5)).await)?, 0);
    assert_eq!(snapshot(&pg).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_owner_deleted_image_loses_its_refs_after_the_window_and_a_hold_keeps_it(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let plain = upload(&pg, &owner, group, 1).await?;
    let reported = upload(&pg, &owner, group, 1).await?;
    let early = upload(&pg, &owner, group, 1).await?;
    for (seq, image) in [&plain, &reported, &early].into_iter().enumerate() {
        assert!(attach(&pg, image.id, group, seq as i64 + 1).await?);
    }
    report(&pg, group, &[reported.id]).await?;
    for image in [&plain, &reported, &early] {
        assert!(owner_delete(&pg, image.id, 15).await?);
    }
    pass_window(&pg, plain.id).await?;
    pass_window(&pg, reported.id).await?;
    assert_eq!(usage(&pg, &owner).await?, Usage::default());

    assert_eq!(ok(withdrawn::run(&chat, &mut Budget::new(5)).await)?, 2);

    assert_eq!(refs(&pg, plain.id).await?, 0);
    assert_eq!(statuses(&pg, &plain).await?, deleting());
    assert_eq!(refs(&pg, reported.id).await?, 0);
    assert_eq!(
        statuses(&pg, &reported).await?,
        vec![Some("held".to_owned()); 2]
    );
    assert_eq!(refs(&pg, early.id).await?, 1, "the restore window is open");
    assert_eq!(usage(&pg, &owner).await?, Usage::default());
    let before = snapshot(&pg).await?;
    assert_eq!(ok(withdrawn::run(&chat, &mut Budget::new(5)).await)?, 0);
    assert_eq!(snapshot(&pg).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn holds_end_at_until_or_when_the_report_is_handled(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let lapsed = upload(&pg, &owner, group, 1).await?;
    let handled = upload(&pg, &owner, group, 1).await?;
    let open = upload(&pg, &owner, group, 1).await?;
    for (seq, image) in [&lapsed, &handled, &open].into_iter().enumerate() {
        attach(&pg, image.id, group, seq as i64 + 1).await?;
    }
    let first = report(&pg, group, &[lapsed.id]).await?;
    let second = report(&pg, group, &[handled.id]).await?;
    report(&pg, group, &[open.id]).await?;
    for seq in 1..=3 {
        redact(&pg, group, seq).await?;
    }
    for image in [&lapsed, &handled, &open] {
        assert_eq!(status(&pg, image.id).await?.as_deref(), Some("held"));
    }
    assert_eq!(
        usage(&pg, &owner).await?,
        Usage::default(),
        "held charges nobody"
    );
    sqlx::query(
        "UPDATE chat.media_holds SET until = now() - interval '1 second' WHERE report_id = $1",
    )
    .bind(first)
    .execute(&pg)
    .await?;
    handle(&pg, second).await?;

    assert_eq!(ok(holds::run(&chat, &mut Budget::new(5)).await)?, 2);

    assert_eq!(statuses(&pg, &lapsed).await?, deleting());
    assert_eq!(statuses(&pg, &handled).await?, deleting());
    assert_eq!(status(&pg, open.id).await?.as_deref(), Some("held"));
    let before = snapshot(&pg).await?;
    assert_eq!(ok(holds::run(&chat, &mut Budget::new(5)).await)?, 0);
    assert_eq!(snapshot(&pg).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_held_row_whose_hold_vanished_without_a_settle_is_freed(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let image = upload(&pg, &owner, group, 1).await?;
    attach(&pg, image.id, group, 1).await?;
    let held = report(&pg, group, &[image.id]).await?;
    redact(&pg, group, 1).await?;
    sqlx::query("DELETE FROM chat.media_holds WHERE report_id = $1")
        .bind(held)
        .execute(&pg)
        .await?;

    assert_eq!(ok(holds::run(&chat, &mut Budget::new(5)).await)?, 1);
    assert_eq!(statuses(&pg, &image).await?, deleting());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_dm_frees_images_only_below_the_lower_floor_of_both_sides(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (a, b) = (user(), user());
    let dm = conversation(&pg, "direct", &a).await?;
    member(&pg, dm, &a, "active").await?;
    member(&pg, dm, &b, "active").await?;
    let mut images = Vec::new();
    for seq in 1..=4 {
        let image = upload(&pg, if seq % 2 == 0 { &a } else { &b }, dm, 1).await?;
        attach(&pg, image.id, dm, seq).await?;
        images.push(image);
    }
    let a_before = usage(&pg, &a).await?;

    hide(&pg, dm, &a, 4).await?;
    assert_eq!(ok(trim::run(&chat, &mut Budget::new(5)).await)?, 0);
    for image in &images {
        assert_eq!(status(&pg, image.id).await?.as_deref(), Some("attached"));
    }
    assert_eq!(usage(&pg, &a).await?, a_before);

    hide(&pg, dm, &b, 2).await?;
    assert_eq!(ok(trim::run(&chat, &mut Budget::new(5)).await)?, 2);
    assert_eq!(statuses(&pg, &images[0]).await?, deleting());
    assert_eq!(statuses(&pg, &images[1]).await?, deleting());
    assert_eq!(
        status(&pg, images[2].id).await?.as_deref(),
        Some("attached")
    );
    assert_eq!(
        status(&pg, images[3].id).await?.as_deref(),
        Some("attached")
    );
    assert_eq!(
        usage(&pg, &a).await?.bytes,
        a_before.bytes - images[1].total()
    );
    let trims: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.media_trim")
        .fetch_one(&pg)
        .await?;
    assert_eq!(trims, 0);
    let before = snapshot(&pg).await?;
    assert_eq!(ok(trim::run(&chat, &mut Budget::new(5)).await)?, 0);
    assert_eq!(snapshot(&pg).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn trim_keeps_the_group_avatar_and_history_a_member_can_still_read(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (owner, late) = (user(), user());
    let group = conversation(&pg, "group", &owner).await?;
    member(&pg, group, &owner, "active").await?;
    member(&pg, group, &late, "active").await?;
    sqlx::query(
        "UPDATE chat.members SET joined_seq = 3 WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(group)
    .bind(&late)
    .execute(&pg)
    .await?;
    let avatar = reserve(&pg, &owner, group, 1, false).await?;
    crate::modules::chat::sweeps::media::fixtures::commit(&pg, &avatar).await?;
    attach(&pg, avatar.id, group, 0).await?;
    let early = upload(&pg, &owner, group, 1).await?;
    attach(&pg, early.id, group, 2).await?;

    hide(&pg, group, &owner, 5).await?;
    ok(trim::run(&chat, &mut Budget::new(5)).await)?;
    assert_eq!(
        status(&pg, early.id).await?.as_deref(),
        Some("attached"),
        "history is visible, so the late member still reads seq 2"
    );

    sqlx::query(
        "UPDATE chat.conversations SET settings = '{\"history_visible\": false}' WHERE id = $1",
    )
    .bind(group)
    .execute(&pg)
    .await?;
    hide(&pg, group, &owner, 5).await?;
    assert_eq!(ok(trim::run(&chat, &mut Budget::new(5)).await)?, 1);
    assert_eq!(statuses(&pg, &early).await?, deleting());
    assert_eq!(status(&pg, avatar.id).await?.as_deref(), Some("attached"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_trim_cut_by_the_budget_resumes_and_keeps_its_queue_entry(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (a, b) = (user(), user());
    let dm = conversation(&pg, "direct", &a).await?;
    member(&pg, dm, &a, "active").await?;
    member(&pg, dm, &b, "active").await?;
    let image = upload(&pg, &a, dm, 1).await?;
    attach(&pg, image.id, dm, 1).await?;
    hide(&pg, dm, &a, 9).await?;
    hide(&pg, dm, &b, 9).await?;

    assert_eq!(ok(trim::run(&chat, &mut Budget::new(0)).await)?, 0);
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.media_trim")
        .fetch_one(&pg)
        .await?;
    assert_eq!(queued, 1);
    assert_eq!(status(&pg, image.id).await?.as_deref(), Some("attached"));

    assert_eq!(ok(trim::run(&chat, &mut Budget::new(1)).await)?, 1);
    assert_eq!(statuses(&pg, &image).await?, deleting());
    assert_eq!(usage(&pg, &a).await?, Usage::default());
    Ok(())
}
