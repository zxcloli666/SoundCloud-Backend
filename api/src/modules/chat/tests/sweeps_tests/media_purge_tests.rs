use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, member, messages, room, state, user};
use crate::modules::chat::conversations::remove;
use crate::modules::chat::members::{access, changes};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    Upload, Usage, attach, handle, ok, owner_delete, pass_window, queue_trim, redact, refs, report,
    reserve, rows_of, snapshot, status, statuses, upload, usage,
};
use crate::modules::chat::sweeps::media::{self, BATCH_REFS, holds, trim, withdrawn};
use crate::modules::chat::sweeps::{expired, purge};

fn deleting() -> Vec<Option<String>> {
    vec![Some("deleting".to_owned()); 2]
}

async fn group_of(pg: &PgPool, owner: &str, others: &[&str]) -> anyhow::Result<Uuid> {
    let group = conversation(pg, "group", owner).await?;
    member(pg, group, owner, "active").await?;
    sqlx::query(
        "UPDATE chat.members SET role = 'owner' WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(group)
    .bind(owner)
    .execute(pg)
    .await?;
    for other in others {
        member(pg, group, other, "active").await?;
    }
    sqlx::query("UPDATE chat.conversations SET member_count = $2 WHERE id = $1")
        .bind(group)
        .bind(1 + others.len() as i32)
        .execute(pg)
        .await?;
    Ok(group)
}

async fn post(pg: &PgPool, conversation: Uuid, seq: i64, image: &Upload) -> anyhow::Result<()> {
    messages(pg, conversation, seq..=seq, None).await?;
    anyhow::ensure!(
        attach(pg, image.id, conversation, seq).await?,
        "not attached"
    );
    Ok(())
}

async fn steps(chat: &crate::modules::chat::ChatState) -> anyhow::Result<u64> {
    let mut changed = 0;
    changed += ok(media::expire::run(chat, &mut Budget::new(5)).await)?;
    changed += ok(trim::run(chat, &mut Budget::new(5)).await)?;
    changed += ok(withdrawn::run(chat, &mut Budget::new(5)).await)?;
    changed += ok(holds::run(chat, &mut Budget::new(5)).await)?;
    changed += ok(expired::run_once(chat).await)?;
    changed += ok(purge::run_once(chat).await)?;
    Ok(changed)
}

async fn purge_all(chat: &crate::modules::chat::ChatState) -> anyhow::Result<()> {
    for _ in 0..5 {
        ok(purge::run_once(chat).await)?;
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_purge_cut_between_ref_batches_resumes_and_releases_each_image_once(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let doomed = group_of(&pg, &owner, &[]).await?;
    let kept = group_of(&pg, &owner, &[]).await?;
    messages(&pg, doomed, 1..=BATCH_REFS + 1, None).await?;
    let wide = upload(&pg, &owner, doomed, 1).await?;
    let last = upload(&pg, &owner, doomed, 1).await?;
    let other = upload(&pg, &owner, kept, 1).await?;
    attach(&pg, other.id, kept, 1).await?;
    attach(&pg, wide.id, doomed, 1).await?;
    sqlx::query(
        "INSERT INTO chat.media_refs (media_id, conversation_id, seq)
         SELECT $1, $2, seq FROM generate_series(2::int8, $3::int8) AS seq",
    )
    .bind(wide.id)
    .bind(doomed)
    .bind(BATCH_REFS)
    .execute(&pg)
    .await?;
    attach(&pg, last.id, doomed, BATCH_REFS + 1).await?;
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(doomed)
        .execute(&pg)
        .await?;

    assert!(!ok(purge::conversation(
        &chat,
        doomed,
        &mut Budget::new(1)
    )
    .await)?);
    assert_eq!(statuses(&pg, &wide).await?, deleting());
    assert_eq!(status(&pg, last.id).await?.as_deref(), Some("attached"));
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chat.messages WHERE conversation_id = $1")
            .bind(doomed)
            .fetch_one(&pg)
            .await?;
    assert_eq!(left, BATCH_REFS + 1, "messages wait for the refs");
    assert_eq!(
        usage(&pg, &owner).await?.bytes,
        other.total() + last.total()
    );

    assert!(!ok(purge::conversation(
        &chat,
        doomed,
        &mut Budget::new(1)
    )
    .await)?);
    assert!(ok(purge::conversation(
        &chat,
        doomed,
        &mut Budget::new(10)
    )
    .await)?);
    assert_eq!(statuses(&pg, &last).await?, deleting());
    assert_eq!(
        usage(&pg, &owner).await?,
        Usage {
            bytes: other.total(),
            reserved: 0,
            files: 1
        }
    );
    assert_eq!(status(&pg, other.id).await?.as_deref(), Some("attached"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_ended_room_frees_every_image_and_a_reported_one_waits_for_the_admin(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    chat.rooms.install_chat(&chat);
    let senders = [user(), user(), user()];
    let room_chat = conversation(&pg, "room", &senders[0]).await?;
    let elsewhere = group_of(&pg, &senders[0], &[]).await?;
    let mut before = Vec::new();
    for sender in &senders {
        let keep = upload(&pg, sender, elsewhere, 1).await?;
        attach(&pg, keep.id, elsewhere, 1).await?;
        before.push(usage(&pg, sender).await?);
    }
    let mut posted = Vec::new();
    let mut seq = 0;
    for sender in &senders {
        for _ in 0..10 {
            seq += 1;
            let image = upload(&pg, sender, room_chat, 1).await?;
            post(&pg, room_chat, seq, &image).await?;
            posted.push(image);
        }
    }
    let unsent = upload(&pg, &senders[1], room_chat, 1).await?;
    let pending = reserve(&pg, &senders[2], room_chat, 1, true).await?;
    let reported = posted[4].clone();
    let case = report(&pg, room_chat, &[reported.id]).await?;
    let live = room(&chat, &senders[0], room_chat).await?;

    assert!(
        chat.rooms
            .end(&live.code)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
    );
    assert_eq!(statuses(&pg, &unsent).await?, deleting());
    assert_eq!(statuses(&pg, &pending).await?, deleting());
    purge_all(&chat).await?;

    for image in &posted {
        assert_eq!(refs(&pg, image.id).await?, 0);
        let expected = if image.id == reported.id {
            "held"
        } else {
            "deleting"
        };
        assert_eq!(status(&pg, image.id).await?.as_deref(), Some(expected));
    }
    for (sender, was) in senders.iter().zip(&before) {
        assert_eq!(usage(&pg, sender).await?, *was);
    }
    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.conversations WHERE id = $1")
        .bind(room_chat)
        .fetch_one(&pg)
        .await?;
    assert_eq!(gone, 0);

    handle(&pg, case).await?;
    ok(holds::run(&chat, &mut Budget::new(5)).await)?;
    assert_eq!(statuses(&pg, &reported).await?, deleting());
    let live_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.media WHERE origin_conversation = $1 AND status <> 'deleting'",
    )
    .bind(room_chat)
    .fetch_one(&pg)
    .await?;
    assert_eq!(live_rows, 0);
    assert_eq!(rows_of(&pg, room_chat).await?, 64);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_room_shredded_before_a_late_commit_still_frees_it_on_purge(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let host = user();
    let room_chat = conversation(&pg, "room", &host).await?;
    let late = reserve(&pg, &host, room_chat, 1, true).await?;
    assert!(ok(purge::shred(&chat, room_chat).await)?);
    assert_eq!(statuses(&pg, &late).await?, deleting());
    let racing = reserve(&pg, &host, room_chat, 1, true).await?;
    assert!(crate::modules::chat::sweeps::media::fixtures::commit(&pg, &racing).await?);

    purge_all(&chat).await?;

    assert_eq!(statuses(&pg, &racing).await?, deleting());
    assert_eq!(usage(&pg, &host).await?, Usage::default());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_group_frees_images_only_when_it_is_deleted_or_empty(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (owner, guest) = (user(), user());
    let group = group_of(&pg, &owner, &[&guest]).await?;
    let image = upload(&pg, &owner, group, 1).await?;
    post(&pg, group, 1, &image).await?;
    let lonely = group_of(&pg, &guest, &[]).await?;
    let last = upload(&pg, &guest, lonely, 1).await?;
    post(&pg, lonely, 1, &last).await?;

    let guest_access = ok(access::load(&pg, group, &guest).await)?;
    ok(changes::leave(&pg, &guest_access).await)?;
    queue_trim(&pg, group).await?;
    ok(trim::run(&chat, &mut Budget::new(5)).await)?;
    purge_all(&chat).await?;
    assert_eq!(status(&pg, image.id).await?.as_deref(), Some("attached"));

    let owner_access = ok(access::load(&pg, group, &owner).await)?;
    assert!(ok(remove::for_everyone(&pg, &owner_access).await)?);
    let alone = ok(access::load(&pg, lonely, &guest).await)?;
    assert!(ok(changes::leave(&pg, &alone).await)?.deleted);
    purge_all(&chat).await?;

    assert_eq!(statuses(&pg, &image).await?, deleting());
    assert_eq!(statuses(&pg, &last).await?, deleting());
    assert_eq!(usage(&pg, &owner).await?, Usage::default());
    assert_eq!(usage(&pg, &guest).await?, Usage::default());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_forward_outlives_the_original_until_its_uploader_deletes_it(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (a, b) = (user(), user());
    let dm = conversation(&pg, "direct", &a).await?;
    member(&pg, dm, &a, "active").await?;
    member(&pg, dm, &b, "active").await?;
    let group = group_of(&pg, &b, &[&a]).await?;
    let image = upload(&pg, &a, dm, 1).await?;
    post(&pg, dm, 1, &image).await?;
    attach(&pg, image.id, group, 1).await?;
    for _ in 0..4 {
        let other = group_of(&pg, &b, &[&a]).await?;
        attach(&pg, image.id, other, 1).await?;
    }
    assert_eq!(
        usage(&pg, &a).await?,
        Usage {
            bytes: image.total(),
            reserved: 0,
            files: 1
        },
        "five forwards count once"
    );

    redact(&pg, dm, 1).await?;
    let dm_access = ok(access::load(&pg, dm, &a).await)?;
    assert!(ok(remove::for_everyone(&pg, &dm_access).await)?);
    purge_all(&chat).await?;
    assert_eq!(status(&pg, image.id).await?.as_deref(), Some("attached"));
    assert_eq!(refs(&pg, image.id).await?, 5);

    assert!(owner_delete(&pg, image.id, 15).await?);
    assert_eq!(usage(&pg, &a).await?, Usage::default());
    pass_window(&pg, image.id).await?;
    ok(withdrawn::run(&chat, &mut Budget::new(5)).await)?;
    assert_eq!(refs(&pg, image.id).await?, 0);
    assert_eq!(statuses(&pg, &image).await?, deleting());
    assert_eq!(usage(&pg, &a).await?, Usage::default());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_disappearing_post_takes_its_image_unless_it_was_forwarded(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let dm = conversation(&pg, "direct", &owner).await?;
    member(&pg, dm, &owner, "active").await?;
    let group = group_of(&pg, &owner, &[]).await?;
    messages(&pg, dm, 1..=2, Some("-1 second")).await?;
    messages(&pg, dm, 3..=3, Some("1 hour")).await?;
    let gone = upload(&pg, &owner, dm, 1).await?;
    let forwarded = upload(&pg, &owner, dm, 1).await?;
    let fresh = upload(&pg, &owner, dm, 1).await?;
    attach(&pg, gone.id, dm, 1).await?;
    attach(&pg, forwarded.id, dm, 2).await?;
    attach(&pg, forwarded.id, group, 1).await?;
    attach(&pg, fresh.id, dm, 3).await?;

    assert_eq!(ok(expired::run_once(&chat).await)?, 2);

    assert_eq!(statuses(&pg, &gone).await?, deleting());
    assert_eq!(
        status(&pg, forwarded.id).await?.as_deref(),
        Some("attached")
    );
    assert_eq!(refs(&pg, forwarded.id).await?, 1);
    assert_eq!(status(&pg, fresh.id).await?.as_deref(), Some("attached"));
    assert_eq!(
        usage(&pg, &owner).await?.bytes,
        forwarded.total() + fresh.total()
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn every_media_step_run_twice_changes_nothing_the_second_time(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (a, b) = (user(), user());
    let dm = conversation(&pg, "direct", &a).await?;
    member(&pg, dm, &a, "active").await?;
    member(&pg, dm, &b, "active").await?;
    let room_chat = conversation(&pg, "room", &a).await?;
    for seq in 1..=6 {
        let image = upload(&pg, &a, dm, 1).await?;
        post(&pg, dm, seq, &image).await?;
        let room_image = upload(&pg, &b, room_chat, 1).await?;
        post(&pg, room_chat, seq, &room_image).await?;
    }
    let stale = reserve(&pg, &a, dm, 1, true).await?;
    crate::modules::chat::sweeps::media::fixtures::age(&pg, stale.id).await?;
    let held = upload(&pg, &a, dm, 1).await?;
    attach(&pg, held.id, dm, 7).await?;
    report(&pg, dm, &[held.id]).await?;
    owner_delete(&pg, held.id, 15).await?;
    pass_window(&pg, held.id).await?;
    crate::modules::chat::sweeps::media::fixtures::hide(&pg, dm, &a, 3).await?;
    crate::modules::chat::sweeps::media::fixtures::hide(&pg, dm, &b, 4).await?;
    ok(purge::shred(&chat, room_chat).await)?;

    let first = steps(&chat).await?;
    let after_first = snapshot(&pg).await?;
    let second = steps(&chat).await?;

    assert!(first > 0);
    assert_eq!(second, 0);
    assert_eq!(snapshot(&pg).await?, after_first);
    assert_eq!(statuses(&pg, &stale).await?, deleting());
    assert_eq!(status(&pg, held.id).await?.as_deref(), Some("held"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_forward_racing_the_purge_of_its_last_ref_never_leaves_a_ref_on_a_dead_row(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let target = group_of(&pg, &owner, &[]).await?;
    for round in 0..20 {
        let doomed = group_of(&pg, &owner, &[]).await?;
        let image = upload(&pg, &owner, doomed, 1).await?;
        post(&pg, doomed, 1, &image).await?;
        sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
            .bind(doomed)
            .execute(&pg)
            .await?;
        let forwards: Vec<_> = (0..4)
            .map(|index| {
                let pg = pg.clone();
                tokio::spawn(
                    async move { attach(&pg, image.id, target, round * 10 + index + 1).await },
                )
            })
            .collect();
        let purging = {
            let chat = chat.clone();
            tokio::spawn(
                async move { purge::conversation(&chat, doomed, &mut Budget::new(10)).await },
            )
        };
        let mut landed = 0;
        for forward in forwards {
            if forward.await?? {
                landed += 1;
            }
        }
        assert!(ok(purging.await?)?);
        let state = status(&pg, image.id).await?;
        let left = refs(&pg, image.id).await?;
        assert_eq!(left, landed, "round {round}");
        match left {
            0 => assert_eq!(state.as_deref(), Some("deleting"), "round {round}"),
            _ => assert_eq!(state.as_deref(), Some("attached"), "round {round}"),
        }
    }
    let charged: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(bytes), 0)::bigint FROM chat.media
         WHERE owner_user = $1 AND status IN ('uploaded', 'attached')",
    )
    .bind(&owner)
    .fetch_one(&pg)
    .await?;
    assert_eq!(usage(&pg, &owner).await?.bytes, charged);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_room_shred_killed_mid_way_keeps_room_and_images_in_step(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let host = user();
    let room_chat = conversation(&pg, "room", &host).await?;
    let outside = group_of(&pg, &host, &[]).await?;
    let unsent = upload(&pg, &host, room_chat, 1).await?;
    let mut holder = pg.begin().await?;
    sqlx::query("SELECT id FROM chat.media WHERE id = $1 FOR UPDATE")
        .bind(unsent.id)
        .execute(&mut *holder)
        .await?;

    let killed = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        purge::shred(&chat, room_chat),
    )
    .await;
    assert!(
        killed.is_err(),
        "the shred waits on the image lock and is killed"
    );
    holder.rollback().await?;
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
         WHERE datname = current_database() AND pid <> pg_backend_pid()",
    )
    .execute(&pg)
    .await?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let others: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND pid <> pg_backend_pid() AND state <> 'idle'",
        )
        .fetch_one(&pg)
        .await?;
        if others == 0 {
            break;
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "the killed shred lingers"
        );
        tokio::task::yield_now().await;
    }
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chat.conversations WHERE id = $1")
            .bind(room_chat)
            .fetch_one(&pg)
            .await?;
    assert!(!deleted, "the killed shred committed nothing");
    assert_eq!(status(&pg, unsent.id).await?.as_deref(), Some("uploaded"));

    assert!(ok(purge::shred(&chat, room_chat).await)?);
    assert_eq!(statuses(&pg, &unsent).await?, deleting());
    assert!(
        !attach(&pg, unsent.id, outside, 1).await?,
        "no room image leaves the room"
    );
    purge_all(&chat).await?;
    assert_eq!(rows_of(&pg, room_chat).await?, 2);
    assert_eq!(statuses(&pg, &unsent).await?, deleting());
    assert!(!attach(&pg, unsent.id, outside, 1).await?);
    assert_eq!(usage(&pg, &host).await?, Usage::default());
    Ok(())
}
