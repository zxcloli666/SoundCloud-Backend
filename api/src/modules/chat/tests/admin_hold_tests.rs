use std::time::Duration;

use sc_chat_wire::msg::Class;
use sqlx::PgPool;
use uuid::Uuid;

use super::cipher::Role;
use super::media::view_with;
use super::media_fixtures::{attached, decision, with_store};
use super::media_support::{
    Row, Upload, conversation_key, image, media_row, meta_envelope, post_envelope, refs_of, send,
    single, status_of, webp,
};
use super::resolve::{AdminDecision, apply};
use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::fixtures::{DEVICE, access_of, outgoing};
use crate::modules::chat::fake_store::FakeStore;
use crate::modules::chat::media::support::call;
use crate::modules::chat::messages::store;
use crate::modules::chat::moderation::testing::{conversation, state};
use crate::modules::chat::reports::{FileReport, file_with};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::{delete, holds};

pub struct Posted {
    pub group: Uuid,
    pub other: Uuid,
    pub report: Uuid,
    pub seq: i64,
    pub images: Vec<(Upload, Upload)>,
}

pub async fn posted(pool: &PgPool, chat: &ChatState) -> Posted {
    let keys = &chat.keks;
    let group = conversation(pool, "group", &[("91", true), ("92", true)]).await;
    let other = conversation(pool, "group", &[("92", true), ("93", true)]).await;
    let ck = conversation_key(pool, keys, group).await;
    let images: Vec<(Upload, Upload)> = ["forwarded", "only-here"]
        .into_iter()
        .map(|tag| {
            (
                Upload::new(Role::Image, &webp(tag, 5_000)),
                Upload::new(Role::Thumb, &webp(&format!("{tag}-thumb"), 600)),
            )
        })
        .collect();
    let seq = send(
        pool,
        group,
        &ck,
        "92",
        Class::Post,
        &post_envelope(
            "two pictures",
            images
                .iter()
                .map(|(full, thumb)| image(full, thumb))
                .collect(),
        ),
    )
    .await
    .seq;
    for (full, thumb) in &images {
        attached(pool, full, Some(thumb), "92", group).await;
        add_ref(pool, full.id, group, seq).await;
    }
    add_ref(pool, images[0].0.id, other, 7).await;
    let report = file_with(
        chat,
        keys,
        "91",
        &FileReport {
            conversation_id: group,
            seq_from: seq,
            seq_to: seq,
            reason: "other".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report")
    .id;
    Posted {
        group,
        other,
        report,
        seq,
        images,
    }
}

pub async fn add_ref(pool: &PgPool, media: Uuid, conversation: Uuid, seq: i64) {
    sqlx::query("INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, $3)")
        .bind(media)
        .bind(conversation)
        .bind(seq)
        .execute(pool)
        .await
        .expect("a ref");
}

pub async fn holds_of(pool: &PgPool, report: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT media_id FROM chat.media_holds
         WHERE report_id = $1 AND until > now() + interval '29 days'
         ORDER BY media_id",
    )
    .bind(report)
    .fetch_all(pool)
    .await
    .expect("holds")
}

pub async fn sweep(chat: &ChatState) {
    holds::run(chat, &mut Budget::new(holds::BATCHES))
        .await
        .expect("the holds step");
    delete::run(chat, &mut Budget::new(delete::CLAIMS))
        .await
        .expect("the delete step");
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_reported_group_avatar_stays_for_the_moderator_after_it_is_replaced(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = with_store(state(pool.clone(), None), &fake.endpoint, "fake", "fake");
    let keys = &chat.keks;
    let group = conversation(&pool, "group", &[("91", true), ("92", true)]).await;
    let ck = conversation_key(&pool, keys, group).await;
    let first = Upload::new(Role::Image, &webp("first-avatar", 900));
    attached(&pool, &first, None, "92", group).await;
    add_ref(&pool, first.id, group, 0).await;
    fake.put(&first.object_key(), first.ciphertext.clone());
    let meta = send(
        &pool,
        group,
        &ck,
        "92",
        Class::Meta,
        &meta_envelope(Some(single(&first, 512, 512))),
    )
    .await;
    let report = file_with(
        &chat,
        keys,
        "91",
        &FileReport {
            conversation_id: group,
            seq_from: meta.seq,
            seq_to: meta.seq,
            reason: "other".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report")
    .id;
    assert_eq!(holds_of(&pool, report).await, vec![first.id]);

    let second = Upload::new(Role::Image, &webp("second-avatar", 700));
    media_row(
        &pool,
        Row {
            upload: &second,
            owner: "91",
            origin: group,
            kind: "avatar",
            parent: None,
            status: "uploaded",
        },
    )
    .await;
    let mut out = outgoing(Class::Meta, 0, None);
    out.media = vec![second.id];
    store::store(
        &pool,
        &access_of(&pool, group, "91").await,
        DEVICE,
        &out,
        None,
    )
    .await
    .expect("a new avatar");
    assert_eq!(refs_of(&pool, group).await, vec![(second.id, 0)]);
    assert_eq!(status_of(&pool, first.id).await.as_deref(), Some("held"));

    sweep(&chat).await;
    assert!(fake.object(&first.object_key()).is_some());
    let shown = view_with(&chat, keys, "owner", report, first.id)
        .await
        .expect("the moderator sees the replaced avatar");
    assert_eq!(shown, first.plain);

    apply(
        &chat,
        &chat.keks,
        "owner",
        report,
        &decision(AdminDecision::Reject),
    )
    .await
    .expect("rejected");
    sweep(&chat).await;
    assert_eq!(status_of(&pool, first.id).await, None);
    assert!(fake.object(&first.object_key()).is_none());
    assert_eq!(
        status_of(&pool, second.id).await.as_deref(),
        Some("attached")
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_reopened_report_holds_again_what_still_exists(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = with_store(state(pool.clone(), None), &fake.endpoint, "fake", "fake");
    let keys = &chat.keks;
    let posted = posted(&pool, &chat).await;
    for (full, thumb) in &posted.images {
        fake.put(&full.object_key(), full.ciphertext.clone());
        fake.put(&thumb.object_key(), thumb.ciphertext.clone());
    }
    let (forwarded, freed) = (&posted.images[0], &posted.images[1]);
    let both = sorted(vec![forwarded.0.id, freed.0.id]);
    assert_eq!(holds_of(&pool, posted.report).await, both);

    let mut uphold = decision(AdminDecision::Uphold);
    uphold.delete_seqs = vec![posted.seq];
    apply(&chat, &chat.keks, "owner", posted.report, &uphold)
        .await
        .expect("upheld");
    assert!(holds_of(&pool, posted.report).await.is_empty());
    assert!(refs_of(&pool, posted.group).await.is_empty());
    assert_eq!(
        refs_of(&pool, posted.other).await,
        vec![(forwarded.0.id, 7)]
    );
    for id in [freed.0.id, freed.1.id] {
        assert_eq!(status_of(&pool, id).await.as_deref(), Some("deleting"));
    }

    let reopened = apply(
        &chat,
        &chat.keks,
        "owner",
        posted.report,
        &decision(AdminDecision::Reopen),
    )
    .await
    .expect("reopened");
    assert_eq!(reopened.state, "open");
    assert_eq!(holds_of(&pool, posted.report).await, both);
    for id in [freed.0.id, freed.1.id] {
        assert_eq!(status_of(&pool, id).await.as_deref(), Some("held"));
    }
    assert_eq!(
        status_of(&pool, forwarded.0.id).await.as_deref(),
        Some("attached")
    );

    sweep(&chat).await;
    for upload in [&freed.0, &freed.1] {
        assert!(fake.object(&upload.object_key()).is_some());
        let shown = view_with(&chat, keys, "owner", posted.report, upload.id)
            .await
            .expect("the reopened report still shows it");
        assert_eq!(shown, upload.plain);
    }

    apply(
        &chat,
        &chat.keks,
        "owner",
        posted.report,
        &decision(AdminDecision::Reject),
    )
    .await
    .expect("rejected");
    sweep(&chat).await;
    for upload in [&freed.0, &freed.1] {
        assert_eq!(status_of(&pool, upload.id).await, None);
        assert!(fake.object(&upload.object_key()).is_none());
    }
    for upload in [&forwarded.0, &forwarded.1] {
        assert_eq!(
            status_of(&pool, upload.id).await.as_deref(),
            Some("attached")
        );
        assert!(fake.object(&upload.object_key()).is_some());
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_committed_avatar_is_held_and_shown_from_its_kept_copy_after_a_later_put(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = with_store(state(pool.clone(), None), &fake.endpoint, "fake", "fake");
    let keys = &chat.keks;
    let group = conversation(&pool, "group", &[("91", true), ("92", true)]).await;
    let ck = conversation_key(&pool, keys, group).await;
    let avatar = Upload::new(Role::Image, &webp("committed-avatar", 900));
    media_row(
        &pool,
        Row {
            upload: &avatar,
            owner: "92",
            origin: group,
            kind: "avatar",
            parent: None,
            status: "reserved",
        },
    )
    .await;
    sqlx::query("UPDATE chat.media SET delete_after = now() + interval '1 hour' WHERE id = $1")
        .bind(avatar.id)
        .execute(&pool)
        .await
        .expect("a live reservation");
    fake.put(&avatar.object_key(), avatar.ciphertext.clone());
    let commit = call(
        &chat,
        "92",
        "POST",
        &format!("/chat/media/{}/commit", avatar.id),
        None,
    )
    .await;
    assert_eq!(commit.0, 200, "{commit:?}");
    fake.put(&avatar.object_key(), vec![0; avatar.ciphertext.len()]);
    sqlx::query("UPDATE chat.media SET status = 'attached', delete_after = NULL WHERE id = $1")
        .bind(avatar.id)
        .execute(&pool)
        .await
        .expect("an attached avatar");
    add_ref(&pool, avatar.id, group, 0).await;
    let meta = send(
        &pool,
        group,
        &ck,
        "92",
        Class::Meta,
        &meta_envelope(Some(single(&avatar, 512, 512))),
    )
    .await;
    let report = file_with(
        &chat,
        keys,
        "91",
        &FileReport {
            conversation_id: group,
            seq_from: meta.seq,
            seq_to: meta.seq,
            reason: "other".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report")
    .id;
    assert_eq!(holds_of(&pool, report).await, vec![avatar.id]);

    let shown = view_with(&chat, keys, "owner", report, avatar.id)
        .await
        .expect("the avatar");
    assert_eq!(shown, avatar.plain);
}
