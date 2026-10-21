use std::time::Duration;

use redis::AsyncCommands;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use super::support::{conversation, deltas, member, ok, state, user};
use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::startup;

async fn touched(pg: &PgPool, id: Uuid, last_seq: i64, ago: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE chat.conversations SET last_seq = $2, last_at = now() - $3::text::interval
         WHERE id = $1",
    )
    .bind(id)
    .bind(last_seq)
    .bind(ago)
    .execute(pg)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_bump_lost_before_a_restart_is_sent_again(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (alice, bob) = (user(), user());
    let mut ids = Vec::new();
    for _ in 0..4 {
        let id = conversation(&pg, "group", &alice).await?;
        member(&pg, id, &alice, "active").await?;
        member(&pg, id, &bob, "active").await?;
        ids.push(id);
    }
    let [lost, bumped, old, deleted] = ids[..] else {
        unreachable!()
    };
    touched(&pg, lost, 5, "1 minute").await?;
    touched(&pg, bumped, 3, "1 minute").await?;
    touched(&pg, old, 4, "20 minutes").await?;
    touched(&pg, deleted, 6, "1 minute").await?;
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(deleted)
        .execute(&pg)
        .await?;
    let mut conn = chat.redis.get().await?;
    let _: () = conn.set(rt_keys::conversation_seq(bumped), 3).await?;

    assert_eq!(ok(startup::rebump(&chat).await)?, 1);

    let seq: Option<i64> = conn.get(rt_keys::conversation_seq(lost)).await?;
    assert_eq!(seq, Some(5));
    for viewer in [&alice, &bob] {
        assert_eq!(
            deltas(&chat, viewer).await?,
            vec![lost.to_string()],
            "{viewer}"
        );
    }
    assert_eq!(
        ok(startup::rebump(&chat).await)?,
        0,
        "a second start finds nothing lagging"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_purge_query_left_by_a_dead_process_is_cancelled(pg: PgPool) -> anyhow::Result<()> {
    let chat = super::support::state(pg.clone());
    let owner = user();
    let doomed = conversation(&pg, "group", &owner).await?;
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(doomed)
        .execute(&pg)
        .await?;
    let old_process = PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*pg.connect_options()).clone())
        .await?;
    let mut holder = pg.begin().await?;
    sqlx::query("LOCK TABLE chat.messages IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await?;
    let stuck = tokio::spawn(async move {
        sqlx::query_file!(
            "queries/chat/sweeps/purge_messages.sql",
            doomed,
            crate::modules::chat::sweeps::purge::BATCH_ROWS
        )
        .execute(&old_process)
        .await
    });
    let mut cancelled = 0;
    for _ in 0..100 {
        cancelled = ok(startup::cancel_orphans(&chat).await)?;
        if cancelled > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(cancelled, 1);
    let outcome = stuck.await?;
    let error = outcome.expect_err("the orphan was cancelled");
    assert!(error.to_string().contains("canceling statement"), "{error}");
    holder.rollback().await?;
    assert_eq!(ok(startup::cancel_orphans(&chat).await)?, 0);
    Ok(())
}
