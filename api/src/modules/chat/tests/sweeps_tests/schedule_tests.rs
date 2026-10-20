use sqlx::PgPool;

use super::support::{conversation, count, member, messages, state_on, user};
use crate::modules::chat::sweeps::schedule::Sweeper;

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
