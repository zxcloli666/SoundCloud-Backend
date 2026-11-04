use std::collections::BTreeSet;

use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, count, deltas, member, messages, ok, state, user};
use crate::modules::chat::sweeps::expired::{self, BATCH_ROWS, CONVERSATIONS_PER_QUERY};

const SEQS: &str = "SELECT count(*) FROM chat.messages WHERE conversation_id = $1";

fn gone_seqs(entries: &[String], conversation: Uuid) -> BTreeSet<i64> {
    let prefix = format!("gone:{conversation}:");
    entries
        .iter()
        .filter_map(|entry| entry.strip_prefix(&prefix))
        .flat_map(|seqs| {
            seqs.split(',')
                .map(|seq| seq.parse::<i64>().expect("a seq"))
        })
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn expired_messages_go_and_live_members_hear_about_it(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (alice, bob, carol) = (user(), user(), user());
    let group = conversation(&pg, "group", &alice).await?;
    member(&pg, group, &alice, "active").await?;
    member(&pg, group, &bob, "invited").await?;
    member(&pg, group, &carol, "left").await?;
    messages(&pg, group, 1..=2, Some("-1 second")).await?;
    messages(&pg, group, 3..=3, Some("1 hour")).await?;
    messages(&pg, group, 4..=4, None).await?;

    assert_eq!(ok(expired::run_once(&chat).await)?, 2);

    let left: Vec<i64> =
        sqlx::query_scalar("SELECT seq FROM chat.messages WHERE conversation_id = $1 ORDER BY seq")
            .bind(group)
            .fetch_all(&pg)
            .await?;
    assert_eq!(left, vec![3, 4]);
    for viewer in [&alice, &bob] {
        assert_eq!(
            gone_seqs(&deltas(&chat, viewer).await?, group),
            BTreeSet::from([1, 2]),
            "{viewer}"
        );
    }
    assert!(
        deltas(&chat, &carol).await?.is_empty(),
        "a member who left hears nothing"
    );

    assert_eq!(
        ok(expired::run_once(&chat).await)?,
        0,
        "a second pass finds nothing"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_large_backlog_goes_in_primary_key_batches(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let dm = conversation(&pg, "direct", &owner).await?;
    member(&pg, dm, &owner, "active").await?;
    let total = 2 * BATCH_ROWS + 1;
    messages(&pg, dm, 1..=total, Some("-1 minute")).await?;
    messages(&pg, dm, total + 1..=total + 1, None).await?;

    assert_eq!(ok(expired::run_once(&chat).await)?, total as u64);

    assert_eq!(count(&pg, SEQS, dm).await?, 1);
    let announced = gone_seqs(&deltas(&chat, &owner).await?, dm);
    assert_eq!(announced, (1..=total).collect::<BTreeSet<i64>>());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn many_chats_in_one_batch_are_all_announced(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let wanted = 2 * CONVERSATIONS_PER_QUERY + 5;
    let mut chats = Vec::new();
    for _ in 0..wanted {
        let owner = user();
        let id = conversation(&pg, "group", &owner).await?;
        member(&pg, id, &owner, "active").await?;
        messages(&pg, id, 1..=1, Some("-1 second")).await?;
        chats.push((id, owner));
    }

    assert_eq!(ok(expired::run_once(&chat).await)?, wanted as u64);

    for (id, owner) in chats {
        assert_eq!(
            gone_seqs(&deltas(&chat, &owner).await?, id),
            BTreeSet::from([1]),
            "{id}"
        );
    }
    Ok(())
}
