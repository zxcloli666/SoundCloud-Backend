use sqlx::PgPool;

use super::hold_tests::{holds_of, posted, sweep};
use super::media_fixtures::decision;
use super::media_support::{refs_of, status_of};
use super::resolve::{AdminDecision, apply};
use super::s3_live::{bucketed, stored};
use crate::modules::chat::ChatState;
use crate::modules::chat::admin::media_support::Upload;

async fn on_store(chat: &ChatState, upload: &Upload) -> bool {
    chat.s3
        .head(1, &upload.object_key())
        .await
        .expect("a HEAD")
        .is_some()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs SeaweedFS at CHAT_TEST_S3_ENDPOINT"]
async fn an_upheld_report_releases_its_holds_and_the_sweep_deletes_the_objects(pool: PgPool) {
    let chat = bucketed(&pool).await;
    let posted = posted(&pool, &chat).await;
    for (full, thumb) in &posted.images {
        stored(&chat, full).await;
        stored(&chat, thumb).await;
    }
    let (forwarded, freed) = (&posted.images[0], &posted.images[1]);
    assert_eq!(holds_of(&pool, posted.report).await.len(), 2);

    let mut uphold = decision(AdminDecision::Uphold);
    uphold.delete_seqs = vec![posted.seq];
    apply(&chat, &chat.keks, "owner", posted.report, &uphold)
        .await
        .expect("upheld");
    assert!(holds_of(&pool, posted.report).await.is_empty());
    assert!(refs_of(&pool, posted.group).await.is_empty());
    for upload in [&freed.0, &freed.1] {
        assert_eq!(
            status_of(&pool, upload.id).await.as_deref(),
            Some("deleting")
        );
        assert!(on_store(&chat, upload).await);
    }

    sweep(&chat).await;
    for upload in [&freed.0, &freed.1] {
        assert_eq!(status_of(&pool, upload.id).await, None);
        assert!(!on_store(&chat, upload).await, "the object left S3");
    }
    for upload in [&forwarded.0, &forwarded.1] {
        assert_eq!(
            status_of(&pool, upload.id).await.as_deref(),
            Some("attached")
        );
        assert!(on_store(&chat, upload).await, "the forward keeps it");
    }
}
