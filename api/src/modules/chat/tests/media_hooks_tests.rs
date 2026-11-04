use sc_chat_wire::msg::Class;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::fixtures::{
    DEVICE, access_of, dm, group, key_epoch, outgoing,
};
use crate::modules::chat::media::hooks;
use crate::modules::chat::media::refs;
use crate::modules::chat::media::rows::{Seed, refs as refs_of, seed, status, usage};
use crate::modules::chat::media::support::{IMAGE, THUMB, bench, until_blocked, user};
use crate::modules::chat::messages::{redact, store};
use crate::modules::chat::model::{MemberState, MessageRow};
use crate::modules::chat::moderation::actions::{self, Closing};

const TOTAL: i64 = (IMAGE + THUMB) as i64;

async fn post(chat: &ChatState, conversation: Uuid, user: &str, media: &[Uuid]) -> MessageRow {
    let access = access_of(&chat.pg, conversation, user).await;
    let mut out = outgoing(Class::Post, key_epoch(&chat.pg, conversation).await, None);
    out.media = media.to_vec();
    refs::admit_send(chat, &access, &out)
        .await
        .expect("admitted media");
    store::store(&chat.pg, &access, DEVICE, &out, None)
        .await
        .expect("a post")
}

async fn report(
    pool: &PgPool,
    conversation: Uuid,
    target: &str,
    seq_from: i64,
    seq_to: i64,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.reports (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight)
         VALUES ($1, 'reporter', $2, $3, $4, $5, 'spam', 1)",
    )
    .bind(id)
    .bind(target)
    .bind(conversation)
    .bind(seq_from)
    .bind(seq_to)
    .execute(pool)
    .await
    .expect("a report");
    let mut tx = pool.begin().await.expect("a tx");
    hooks::hold_for_report(&mut tx, id, conversation, seq_from, seq_to)
        .await
        .expect("holds");
    tx.commit().await.expect("a commit");
    id
}

async fn holds(pool: &PgPool, report: Uuid) -> Vec<(Uuid, bool)> {
    sqlx::query_as(
        "SELECT media_id, until BETWEEN now() + interval '29 days' AND now() + interval '31 days'
         FROM chat.media_holds WHERE report_id = $1 ORDER BY media_id",
    )
    .bind(report)
    .fetch_all(pool)
    .await
    .expect("holds")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_redact_releases_the_quota_once_and_a_forward_keeps_the_image(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, friend) = (user(), user());
    let direct = dm(&pool, &owner, &friend, MemberState::Active).await;
    let (gone, _) = seed(&pool, Seed::image(&owner, direct, "uploaded")).await;
    let (kept, _) = seed(&pool, Seed::image(&owner, direct, "uploaded")).await;
    let (forwarded, _) = seed(&pool, Seed::image(&owner, direct, "uploaded")).await;
    let first = post(&chat, direct, &owner, &[gone, forwarded]).await;
    post(&chat, direct, &owner, &[kept]).await;
    let shared = group(&pool, &friend, &[]).await;
    post(&chat, shared, &friend, &[forwarded]).await;
    assert_eq!(usage(&pool, &owner).await, (3 * TOTAL, 0, 3));
    let access = access_of(&pool, direct, &owner).await;
    for _ in 0..3 {
        redact::for_everyone(&pool, &access, first.seq)
            .await
            .expect("a redact");
    }
    assert_eq!(status(&pool, gone).await.as_deref(), Some("deleting"));
    assert_eq!(status(&pool, forwarded).await.as_deref(), Some("attached"));
    let left: Vec<Uuid> = refs_of(&pool, forwarded)
        .await
        .into_iter()
        .map(|(conversation, _)| conversation)
        .collect();
    assert_eq!(left, [shared]);
    assert_eq!(usage(&pool, &owner).await, (2 * TOTAL, 0, 2));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_report_holds_its_images_and_a_removal_releases_the_quota_once(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, member) = (user(), user());
    let group = group(&pool, &owner, &[(&member, MemberState::Active)]).await;
    let (held, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (outside, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let reported = post(&chat, group, &owner, &[held]).await;
    let later = post(&chat, group, &owner, &[outside]).await;
    let id = report(&pool, group, &owner, reported.seq, reported.seq).await;
    assert_eq!(holds(&pool, id).await, [(held, true)]);
    for _ in 0..3 {
        actions::remove_messages(&chat, group, &owner, &[reported.seq])
            .await
            .expect("a removal");
    }
    assert_eq!(status(&pool, held).await.as_deref(), Some("held"));
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    assert_eq!(status(&pool, outside).await.as_deref(), Some("attached"));
    assert!(
        actions::close_open(&chat, id, Closing::Actioned, "admin")
            .await
            .expect("a close")
    );
    assert!(holds(&pool, id).await.is_empty());
    assert_eq!(status(&pool, held).await.as_deref(), Some("deleting"));
    assert!(
        !actions::close_open(&chat, id, Closing::Actioned, "admin")
            .await
            .expect("a second close")
    );
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    assert!(later.seq > reported.seq);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_hold_release_keeps_rows_another_report_still_holds(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (image, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let posted = post(&chat, group, &owner, &[image]).await;
    let first = report(&pool, group, &owner, posted.seq, posted.seq).await;
    let second = report(&pool, group, &owner, 1, posted.seq + 5).await;
    let access = access_of(&pool, group, &owner).await;
    redact::for_everyone(&pool, &access, posted.seq)
        .await
        .expect("a redact");
    assert_eq!(status(&pool, image).await.as_deref(), Some("held"));
    let mut tx = pool.begin().await.expect("a tx");
    hooks::release_holds(&mut tx, first)
        .await
        .expect("released");
    tx.commit().await.expect("a commit");
    assert_eq!(status(&pool, image).await.as_deref(), Some("held"));
    let mut tx = pool.begin().await.expect("a tx");
    hooks::release_holds(&mut tx, second)
        .await
        .expect("released");
    tx.commit().await.expect("a commit");
    assert_eq!(status(&pool, image).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_trim_is_queued_once_per_conversation(pool: PgPool) {
    let conversation = Uuid::new_v4();
    hooks::queue_trim(&pool, conversation)
        .await
        .expect("queued");
    let first: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT queued_at FROM chat.media_trim WHERE conversation_id = $1")
            .bind(conversation)
            .fetch_one(&pool)
            .await
            .expect("a queued trim");
    hooks::queue_trim(&pool, conversation)
        .await
        .expect("queued again");
    let rows: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT queued_at FROM chat.media_trim WHERE conversation_id = $1")
            .bind(conversation)
            .fetch_all(&pool)
            .await
            .expect("queued trims");
    assert_eq!(rows.len(), 1);
    assert!(rows[0] >= first);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_redact_racing_a_report_waits_and_leaves_the_image_held(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (image, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let posted = post(&chat, group, &owner, &[image]).await;
    let id = Uuid::now_v7();
    let mut filing = pool.begin().await.expect("a tx");
    sqlx::query(
        "INSERT INTO chat.reports (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight)
         VALUES ($1, 'reporter', $2, $3, $4, $4, 'spam', 1)",
    )
    .bind(id)
    .bind(&owner)
    .bind(group)
    .bind(posted.seq)
    .execute(&mut *filing)
    .await
    .expect("a report");
    hooks::hold_for_report(&mut filing, id, group, posted.seq, posted.seq)
        .await
        .expect("holds");
    let redacting = {
        let pool = pool.clone();
        let access = access_of(&pool, group, &owner).await;
        tokio::spawn(async move { redact::for_everyone(&pool, &access, posted.seq).await })
    };
    until_blocked(&pool).await;
    assert!(!redacting.is_finished());
    filing.commit().await.expect("a commit");
    redacting.await.expect("a task").expect("a redact");
    assert_eq!(status(&pool, image).await.as_deref(), Some("held"));
    assert_eq!(holds(&pool, id).await, [(image, true)]);
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}
