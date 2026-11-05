use sc_chat_wire::msg::Class;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::cipher::Role;
use super::media_fixtures::{attached, decision, group_report, set_status};
use super::media_support::{
    Upload, conversation_key, hold, image, post_envelope, refs_of, send, status_of, webp,
};
use super::resolve::{AdminDecision, apply};
use super::restore::restore_with;
use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::testing::{audit_actions, body_of, conversation, state};
use crate::modules::chat::reports::{FileReport, file_with};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restore_reattaches_every_live_image_and_never_resurrects(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let dm = conversation(&pool, "direct", &[("71", true), ("72", true)]).await;
    let other = conversation(&pool, "group", &[("72", true), ("73", true)]).await;
    let room = conversation(&pool, "room", &[("72", true)]).await;
    let ck = conversation_key(&pool, keys, dm).await;
    let uploads: Vec<(Upload, Upload)> = (0..8)
        .map(|at| {
            (
                Upload::new(Role::Image, &webp(&format!("image-{at}"), 2_000)),
                Upload::new(Role::Thumb, &webp(&format!("thumb-{at}"), 300)),
            )
        })
        .collect();
    let seq = send(
        &pool,
        dm,
        &ck,
        "72",
        Class::Post,
        &post_envelope(
            "eight",
            uploads
                .iter()
                .map(|(full, thumb)| image(full, thumb))
                .collect(),
        ),
    )
    .await
    .seq;
    let shapes = [
        ("72", dm, "attached"),
        ("72", dm, "uploaded"),
        ("72", dm, "held"),
        ("72", dm, "deleting"),
        ("72", dm, "lost"),
        ("73", other, "attached"),
        ("72", room, "attached"),
    ];
    for ((full, thumb), (owner, origin, _)) in uploads.iter().zip(shapes) {
        attached(&pool, full, Some(thumb), owner, origin).await;
    }
    for (full, _) in [&uploads[0], &uploads[5], &uploads[6]] {
        sqlx::query(
            "INSERT INTO chat.media_refs (media_id, conversation_id, seq) VALUES ($1, $2, 7)",
        )
        .bind(full.id)
        .bind(other)
        .execute(&pool)
        .await
        .expect("a ref elsewhere");
    }
    let report = file_with(
        &chat,
        keys,
        "71",
        &FileReport {
            conversation_id: dm,
            seq_from: seq,
            seq_to: seq,
            reason: "other".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report")
    .id;
    let mut uphold = decision(AdminDecision::Uphold);
    uphold.delete_seqs = vec![seq];
    apply(&chat, &chat.keks, "owner", report, &uphold)
        .await
        .expect("upheld");
    assert!(body_of(&pool, dm, seq).await.is_none());
    sqlx::query("DELETE FROM chat.media_refs WHERE conversation_id = $1")
        .bind(dm)
        .execute(&pool)
        .await
        .expect("refs dropped with the post");
    for ((full, _), (_, _, status)) in uploads.iter().zip(shapes) {
        set_status(&pool, full.id, status).await;
    }
    hold(&pool, uploads[2].0.id, report).await;
    sqlx::query(
        "INSERT INTO chat.usage (user_id, bytes, files) VALUES ('72', 1048576, 5), ('73', 4096, 1)",
    )
    .execute(&pool)
    .await
    .expect("usage rows");
    let usage_before: Vec<(String, i64, i32)> = usage(&pool).await;

    let restored = restore_with(&chat, keys, "owner", report, &[seq])
        .await
        .expect("restored");
    assert_eq!(restored.restored, vec![seq]);
    assert!(body_of(&pool, dm, seq).await.is_some());
    let back = sorted([0, 1, 2, 5].map(|at| uploads[at].0.id).to_vec());
    let missed = sorted([3, 4, 6, 7].map(|at| uploads[at].0.id).to_vec());
    assert_eq!(restored.media, back);
    assert_eq!(restored.media_missed, missed);
    let expected: Vec<(Uuid, i64)> = back.iter().map(|id| (*id, seq)).collect();
    assert_eq!(refs_of(&pool, dm).await, expected);
    for at in [0, 1, 2, 5] {
        for id in [uploads[at].0.id, uploads[at].1.id] {
            assert_eq!(
                status_of(&pool, id).await.as_deref(),
                Some("attached"),
                "upload {at}"
            );
        }
    }
    let open_windows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.media
         WHERE (id = ANY($1) OR parent_id = ANY($1))
           AND (delete_after IS NOT NULL OR restorable_until IS NOT NULL)",
    )
    .bind(&back)
    .fetch_one(&pool)
    .await
    .expect("windows");
    assert_eq!(open_windows, 0);
    for (index, status) in [(3, "deleting"), (4, "lost"), (6, "attached")] {
        assert_eq!(
            status_of(&pool, uploads[index].0.id).await.as_deref(),
            Some(status)
        );
    }
    assert_eq!(status_of(&pool, uploads[7].0.id).await, None);
    let held_back = uploads[2].0.bytes() + uploads[2].1.bytes();
    let mut charged = usage_before.clone();
    charged[0].1 += held_back;
    charged[0].2 += 1;
    assert_eq!(
        usage(&pool).await,
        charged,
        "only the held image is charged again"
    );
    let rows = audit_actions(&pool, &audit::report(report)).await;
    let restore = rows
        .iter()
        .find(|row| row.1 == "restore")
        .expect("a restore row");
    assert_eq!(restore.2["media"], json!(back));
    assert_eq!(restore.2["media_missed"], json!(missed));

    let again = restore_with(&chat, keys, "owner", report, &[seq])
        .await
        .expect("a second restore");
    assert!(again.restored.is_empty() && again.media.is_empty());
    assert_eq!(refs_of(&pool, dm).await, expected);
    assert_eq!(usage(&pool).await, charged);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn parallel_restores_put_each_body_and_ref_back_once(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let mut uphold = decision(AdminDecision::Uphold);
    uphold.delete_seqs = vec![posted.post_seq];
    apply(&chat, &chat.keks, "owner", posted.report, &uphold)
        .await
        .expect("upheld");
    sqlx::query("DELETE FROM chat.media_refs WHERE conversation_id = $1")
        .bind(posted.group)
        .execute(&pool)
        .await
        .expect("refs dropped");
    set_status(&pool, posted.images[0].0.id, "held").await;
    set_status(&pool, posted.images[1].0.id, "uploaded").await;
    sqlx::query("INSERT INTO chat.usage (user_id, bytes, files) VALUES ('92', 0, 0)")
        .execute(&pool)
        .await
        .expect("a usage row");

    let seqs = [posted.post_seq];
    let (left, right) = tokio::join!(
        restore_with(&chat, keys, "owner", posted.report, &seqs),
        restore_with(&chat, keys, "owner", posted.report, &seqs),
    );
    let mut restored: Vec<Vec<i64>> = [left, right]
        .into_iter()
        .map(|answer| answer.expect("an answer").restored)
        .collect();
    restored.sort();
    assert_eq!(restored, vec![Vec::new(), vec![posted.post_seq]]);
    let mut expected: Vec<(Uuid, i64)> = posted
        .images
        .iter()
        .map(|(full, _)| (full.id, posted.post_seq))
        .collect();
    expected.sort();
    assert_eq!(refs_of(&pool, posted.group).await, expected);
    assert_eq!(
        status_of(&pool, posted.images[1].1.id).await.as_deref(),
        Some("attached")
    );
    let (full, thumb) = &posted.images[0];
    assert_eq!(
        usage(&pool).await,
        vec![("92".to_owned(), full.bytes() + thumb.bytes(), 1)],
        "the held image is charged once"
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restore_into_a_deleted_conversation_adds_no_refs(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let mut uphold = decision(AdminDecision::Uphold);
    uphold.delete_seqs = vec![posted.post_seq];
    apply(&chat, &chat.keks, "owner", posted.report, &uphold)
        .await
        .expect("upheld");
    sqlx::query("DELETE FROM chat.media_refs WHERE conversation_id = $1")
        .bind(posted.group)
        .execute(&pool)
        .await
        .expect("refs dropped");
    set_status(&pool, posted.images[0].0.id, "attached").await;
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(posted.group)
        .execute(&pool)
        .await
        .expect("a deleted group");

    restore_with(&chat, keys, "owner", posted.report, &[posted.post_seq])
        .await
        .expect("an answer");
    assert!(refs_of(&pool, posted.group).await.is_empty());
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

async fn usage(pool: &PgPool) -> Vec<(String, i64, i32)> {
    sqlx::query_as("SELECT user_id, bytes, files FROM chat.usage ORDER BY user_id")
        .fetch_all(pool)
        .await
        .expect("usage rows")
}
