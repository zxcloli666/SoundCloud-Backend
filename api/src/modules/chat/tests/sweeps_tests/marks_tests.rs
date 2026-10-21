use redis::AsyncCommands;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, member, ok, state, user};
use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::marks::Flusher;

async fn read_seq(pg: &PgPool, conversation: Uuid, user: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT last_read_seq FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation)
    .bind(user)
    .fetch_one(pg)
    .await?)
}

async fn set_read(pg: &PgPool, conversation: Uuid, user: &str, seq: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE chat.members SET last_read_seq = $3 WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation)
    .bind(user)
    .bind(seq)
    .execute(pg)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn read_marks_reach_members_and_never_move_back(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let (alice, bob) = (user(), user());
    let dm = conversation(&pg, "direct", &alice).await?;
    member(&pg, dm, &alice, "active").await?;
    member(&pg, dm, &bob, "active").await?;
    set_read(&pg, dm, &alice, 2).await?;
    set_read(&pg, dm, &bob, 10).await?;
    let marks = rt_keys::read_marks(dm);
    let mut conn = chat.redis.get().await?;
    let _: () = conn
        .hset_multiple(&marks, &[(&alice, 7), (&bob, 5)])
        .await?;
    let _: () = conn.hset(rt_keys::delivered_marks(dm), &alice, 40).await?;
    let mut flusher = Flusher::new();

    assert!(ok(flusher.run_once(&chat).await)? >= 1);
    assert_eq!(read_seq(&pg, dm, &alice).await?, 7);
    assert_eq!(
        read_seq(&pg, dm, &bob).await?,
        10,
        "a mark never moves back"
    );

    let _: () = conn.hset(&marks, &alice, 9).await?;
    ok(flusher.run_once(&chat).await)?;
    assert_eq!(read_seq(&pg, dm, &alice).await?, 9);

    set_read(&pg, dm, &alice, 1).await?;
    ok(flusher.run_once(&chat).await)?;
    assert_eq!(
        read_seq(&pg, dm, &alice).await?,
        1,
        "unchanged marks are not written again"
    );

    assert!(ok(Flusher::new().run_once(&chat).await)? >= 1);
    assert_eq!(
        read_seq(&pg, dm, &alice).await?,
        9,
        "a fresh process flushes everything once"
    );
    let _: i64 = conn.del(&[marks, rt_keys::delivered_marks(dm)]).await?;
    Ok(())
}
