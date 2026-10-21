use redis::AsyncCommands;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, count, key, member, messages, ok, state, user};
use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::purge::{self, BATCH_ROWS};

const MESSAGES: &str = "SELECT count(*) FROM chat.messages WHERE conversation_id = $1";
const KEYS: &str = "SELECT count(*) FROM chat.conversation_keys WHERE conversation_id = $1";
const MEMBERS: &str = "SELECT count(*) FROM chat.members WHERE conversation_id = $1";
const INVITES: &str = "SELECT count(*) FROM chat.invites WHERE conversation_id = $1";
const ROWS: &str = "SELECT count(*) FROM chat.conversations WHERE id = $1";
const REPORTS: &str = "SELECT count(*) FROM chat.reports WHERE conversation_id = $1";

async fn filled(pg: &PgPool, owner: &str, rows: i64) -> anyhow::Result<Uuid> {
    let id = conversation(pg, "group", owner).await?;
    member(pg, id, owner, "active").await?;
    key(pg, id, 0).await?;
    key(pg, id, 1).await?;
    messages(pg, id, 1..=rows, None).await?;
    sqlx::query(
        "INSERT INTO chat.invites (token_hash, conversation_id, created_by, expires_at)
         VALUES (sha256(gen_random_uuid()::text::bytea), $1, $2, now() + interval '1 day')",
    )
    .bind(id)
    .bind(owner)
    .execute(pg)
    .await?;
    sqlx::query(
        "INSERT INTO chat.reports
            (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight, evidence)
         VALUES (gen_random_uuid(), 'reporter', $2, $1, 1, 1, 'spam', 1, '\\x02'::bytea)",
    )
    .bind(id)
    .bind(owner)
    .execute(pg)
    .await?;
    Ok(id)
}

async fn delete(pg: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(id)
        .execute(pg)
        .await?;
    Ok(())
}

async fn everything(pg: &PgPool, id: Uuid) -> anyhow::Result<[i64; 5]> {
    Ok([
        count(pg, MESSAGES, id).await?,
        count(pg, KEYS, id).await?,
        count(pg, MEMBERS, id).await?,
        count(pg, INVITES, id).await?,
        count(pg, ROWS, id).await?,
    ])
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_deleted_conversation_goes_with_all_it_owns(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let doomed = filled(&pg, &owner, 2 * BATCH_ROWS + 1).await?;
    let kept = filled(&pg, &owner, 3).await?;
    delete(&pg, doomed).await?;
    let mut conn = chat.redis.get().await?;
    let _: () = conn.set(rt_keys::conversation_seq(doomed), 7).await?;
    let _: () = conn.hset(rt_keys::read_marks(doomed), &owner, 7).await?;
    let _: () = conn
        .hset(rt_keys::delivered_marks(doomed), &owner, 7)
        .await?;

    assert_eq!(ok(purge::run_once(&chat).await)?, 1);

    assert_eq!(everything(&pg, doomed).await?, [0, 0, 0, 0, 0]);
    assert_eq!(everything(&pg, kept).await?, [3, 2, 1, 1, 1]);
    assert_eq!(
        count(&pg, REPORTS, doomed).await?,
        1,
        "reports keep their evidence copy"
    );
    let left: i64 = conn
        .exists(&[
            rt_keys::conversation_seq(doomed),
            rt_keys::read_marks(doomed),
            rt_keys::delivered_marks(doomed),
        ])
        .await?;
    assert_eq!(left, 0);
    assert_eq!(
        ok(purge::run_once(&chat).await)?,
        0,
        "a second pass is a no-op"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_purge_cut_short_by_its_budget_resumes(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let doomed = filled(&pg, &owner, BATCH_ROWS + 10).await?;
    delete(&pg, doomed).await?;

    let finished = ok(purge::conversation(&chat, doomed, &mut Budget::new(1)).await)?;

    assert!(!finished);
    assert_eq!(everything(&pg, doomed).await?, [10, 0, 1, 1, 1]);
    assert!(ok(
        purge::conversation(&chat, doomed, &mut Budget::new(2)).await
    )?);
    assert_eq!(everything(&pg, doomed).await?, [0, 0, 0, 0, 0]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_live_conversation_is_never_purged(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let live = filled(&pg, &owner, 3).await?;

    assert!(!ok(
        purge::conversation(&chat, live, &mut Budget::new(5)).await
    )?);

    assert_eq!(everything(&pg, live).await?, [3, 2, 1, 1, 1]);
    Ok(())
}
