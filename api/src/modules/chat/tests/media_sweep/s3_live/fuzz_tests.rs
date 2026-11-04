use sqlx::PgPool;

use super::{bench, bucket, days_later, exists, later_today, listing, store};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{drift, ok};
use crate::modules::chat::sweeps::media::{delete, reconcile, recount};
use crate::modules::chat::sweeps::tests::media_quota_tests::{OPS, op, sweep, world};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn a_thousand_random_operations_with_real_objects_end_consistent(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let mut w = world(&pg).await?;
    w.live = true;
    for _ in 0..OPS {
        op(&b.chat, &pg, &mut w).await?;
    }
    for night in [later_today(), days_later(2)] {
        for which in 0..6 {
            sweep(&b.chat, which).await?;
        }
        ok(delete::run(&b.chat, &mut Budget::new(20)).await)?;
        ok(reconcile::run(&b.chat, night, &mut Budget::new(200)).await)?;
        let saved = ok(reconcile::cursor::load(&b.chat).await)?.expect("a cursor");
        assert_eq!((saved.stage, saved.drift), (reconcile::Stage::Done, 0));
        ok(delete::run(&b.chat, &mut Budget::new(20)).await)?;
    }

    assert_eq!(drift(&pg).await?, 0);
    assert_eq!(ok(recount::all(&b.chat).await)?, 0);
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT object_key, status FROM chat.media")
        .fetch_all(&pg)
        .await?;
    let listed = listing(&b.chat, 1).await?;
    for (key, status) in &rows {
        if matches!(status.as_str(), "uploaded" | "attached") {
            assert!(
                exists(&b.chat, 1, key).await?,
                "{status} row lost its object"
            );
        }
    }
    let keys: std::collections::HashSet<&str> = rows.iter().map(|(key, _)| key.as_str()).collect();
    for key in &listed {
        assert!(
            keys.contains(key.as_str()),
            "an object outlived its row and the 24 h"
        );
    }
    Ok(())
}
