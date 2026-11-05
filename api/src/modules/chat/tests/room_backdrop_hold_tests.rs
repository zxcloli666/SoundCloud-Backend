use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::backdrop_support::{Bench, someone};
use super::report_support::{file, holds, report, stage};
use crate::modules::chat::media::support::until_blocked;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_backdrop_hold_waits_for_the_row_and_refuses_one_claimed_for_delete(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, member) = (someone(), someone());
    let stage = stage(&bench, &host, false, &[&member]).await?;
    let (conversation, media) = (stage.conversation, stage.media);

    let mut claim = bench.pg.begin().await?;
    sqlx::query("SELECT id FROM chat.media WHERE id = $1 FOR UPDATE")
        .bind(media)
        .execute(&mut *claim)
        .await?;
    let (filed, claimed) =
        tokio::join!(file(&bench, &member, report(conversation, media)), async {
            until_blocked(&bench.pg).await;
            sqlx::query(
            "UPDATE chat.media SET status = 'deleting', lease_until = now() + interval '5 minutes'
             WHERE id = $1 OR parent_id = $1",
        )
        .bind(media)
        .execute(&mut *claim)
        .await?;
            claim.commit().await
        });
    claimed?;
    assert_eq!(
        filed?,
        (
            StatusCode::CONFLICT,
            json!({"error": "chat_media_missing", "ids": [media]})
        )
    );
    assert!(holds(&bench, media).await?.is_empty());
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.reports WHERE media_id = $1")
        .bind(media)
        .fetch_one(&bench.pg)
        .await?;
    assert_eq!(stored, 0, "a refused hold files nothing");

    sqlx::query(
        "UPDATE chat.media SET status = 'attached', lease_until = NULL WHERE id = $1 OR parent_id = $1",
    )
    .bind(media)
    .execute(&bench.pg)
    .await?;
    let (status, filed) = file(&bench, &member, report(conversation, media)).await?;
    assert_eq!(status, StatusCode::OK, "{filed}");
    assert_eq!(holds(&bench, media).await?.len(), 1);
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}
