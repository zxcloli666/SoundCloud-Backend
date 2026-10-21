use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::time::Instant;

use super::support::{conversation, count, member, messages, state_on, user};
use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::schedule::Sweeper;
use crate::modules::chat::test_support::chat_state;

const MESSAGES: &str = "SELECT count(*) FROM chat.messages WHERE conversation_id = $1";
const SCHEDULE_DB: u8 = 10;

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn sweeps_run_only_while_this_process_leads(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_on(pg.clone(), SCHEDULE_DB);
    let owner = user();
    let dm = conversation(&pg, "direct", &owner).await?;
    member(&pg, dm, &owner, "active").await?;
    messages(&pg, dm, 1..=3, Some("-1 second")).await?;
    let mut sweeper = Sweeper::new();

    sweeper.tick(&chat, false).await;
    assert_eq!(
        count(&pg, MESSAGES, dm).await?,
        3,
        "a follower sweeps nothing"
    );

    sweeper.tick(&chat, true).await;
    assert_eq!(
        count(&pg, MESSAGES, dm).await?,
        0,
        "the leader sweeps at once"
    );

    messages(&pg, dm, 4..=4, Some("-1 second")).await?;
    sweeper.tick(&chat, true).await;
    assert_eq!(
        count(&pg, MESSAGES, dm).await?,
        1,
        "the next pass waits for the sweep's period"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_failed_startup_is_retried_and_does_not_hold_the_sweeps_back(
    pg: PgPool,
) -> anyhow::Result<()> {
    let broken = chat_state(
        pg.clone(),
        crate::redis::connect_url("redis://127.0.0.1:1")?,
        true,
    );
    let chat = state_on(pg.clone(), SCHEDULE_DB);
    let owner = user();
    let dm = conversation(&pg, "direct", &owner).await?;
    member(&pg, dm, &owner, "active").await?;
    messages(&pg, dm, 1..=2, Some("-1 second")).await?;
    let lagging = conversation(&pg, "group", &owner).await?;
    member(&pg, lagging, &owner, "active").await?;
    sqlx::query("UPDATE chat.conversations SET last_seq = 4 WHERE id = $1")
        .bind(lagging)
        .execute(&pg)
        .await?;
    let mut sweeper = Sweeper::new();

    sweeper.tick(&broken, true).await;
    assert!(!sweeper.started, "the startup failed with redis down");
    assert!(sweeper.startup_due > Instant::now(), "and waits to retry");

    sweeper.tick(&chat, true).await;
    assert!(!sweeper.started, "the retry waits for its delay");
    assert_eq!(
        count(&pg, MESSAGES, dm).await?,
        0,
        "the sweeps run while the startup is pending"
    );

    sweeper.startup_due = Instant::now();
    sweeper.tick(&chat, true).await;
    assert!(sweeper.started);
    let mut conn = chat.redis.get().await?;
    let bumped: Option<i64> = conn.get(rt_keys::conversation_seq(lagging)).await?;
    assert_eq!(bumped, Some(4), "the retry rebumped the lagging chat");

    sweeper.tick(&chat, false).await;
    assert!(!sweeper.started, "a lost lead starts over on the next one");
    Ok(())
}
