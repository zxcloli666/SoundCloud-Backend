use sqlx::PgPool;

use super::support::{
    alive, alive_score, conversation, count, deleted_at_set, deltas, key, messages, minutes_ago,
    ok, room, seen, state_on, user,
};
use crate::modules::chat::sweeps::{purge, rooms};
use crate::modules::rooms::model::new_code;
use crate::modules::rooms::store::RoomStore;

const KEYS: &str = "SELECT count(*) FROM chat.conversation_keys WHERE conversation_id = $1";
const ROWS: &str = "SELECT count(*) FROM chat.conversations WHERE id = $1";

async fn lapse(pg: &PgPool, conversation: uuid::Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE chat.conversations SET room_alive_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(conversation)
    .execute(pg)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_room_nobody_polled_for_ten_minutes_ends_with_its_chat(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_on(pg.clone(), 12);
    let host = user();
    let chat_id = conversation(&pg, "room", &host).await?;
    key(&pg, chat_id, 0).await?;
    messages(&pg, chat_id, 1..=3, None).await?;
    let quiet = room(&chat, &host, chat_id).await?;
    alive(&chat, &quiet.code, minutes_ago(11)).await?;
    seen(&chat, &quiet.code, &host, minutes_ago(11)).await?;

    ok(rooms::run_once(&chat).await)?;

    let store = RoomStore::new(chat.redis.clone());
    assert!(
        store
            .load(&quiet.code)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .is_none()
    );
    assert_eq!(alive_score(&chat, &quiet.code).await?, None);
    assert!(deleted_at_set(&pg, chat_id).await?);
    assert_eq!(
        count(&pg, KEYS, chat_id).await?,
        0,
        "the keys are shredded at once"
    );
    assert!(
        deltas(&chat, &host).await?.contains(&chat_id.to_string()),
        "the last members hear that the chat is gone"
    );

    ok(purge::run_once(&chat).await)?;
    assert_eq!(count(&pg, ROWS, chat_id).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_room_with_a_recent_poll_stays(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_on(pg.clone(), 13);
    let host = user();
    let chat_id = conversation(&pg, "room", &host).await?;
    let busy = room(&chat, &host, chat_id).await?;
    alive(&chat, &busy.code, minutes_ago(11)).await?;
    seen(&chat, &busy.code, &host, minutes_ago(1)).await?;
    lapse(&pg, chat_id).await?;

    ok(rooms::run_once(&chat).await)?;

    let store = RoomStore::new(chat.redis.clone());
    assert!(
        store
            .load(&busy.code)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .is_some()
    );
    assert!(!deleted_at_set(&pg, chat_id).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_lapsed_room_chat_ends_its_quiet_room(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_on(pg.clone(), 14);
    let host = user();
    let chat_id = conversation(&pg, "room", &host).await?;
    key(&pg, chat_id, 0).await?;
    let quiet = room(&chat, &host, chat_id).await?;
    lapse(&pg, chat_id).await?;

    ok(rooms::run_once(&chat).await)?;

    let store = RoomStore::new(chat.redis.clone());
    assert!(
        store
            .load(&quiet.code)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .is_none()
    );
    assert!(deleted_at_set(&pg, chat_id).await?);
    assert_eq!(count(&pg, KEYS, chat_id).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_room_chat_whose_room_vanished_is_shredded(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_on(pg.clone(), 15);
    let host = user();
    let orphan = conversation(&pg, "room", &host).await?;
    key(&pg, orphan, 0).await?;
    sqlx::query("UPDATE chat.conversations SET room_ref = $2 WHERE id = $1")
        .bind(orphan)
        .bind(new_code())
        .execute(&pg)
        .await?;
    lapse(&pg, orphan).await?;
    let alive_chat = conversation(&pg, "room", &host).await?;
    key(&pg, alive_chat, 0).await?;

    ok(rooms::run_once(&chat).await)?;

    assert!(deleted_at_set(&pg, orphan).await?);
    assert_eq!(count(&pg, KEYS, orphan).await?, 0);
    assert!(
        !deleted_at_set(&pg, alive_chat).await?,
        "a chat inside its window stays"
    );
    assert_eq!(count(&pg, KEYS, alive_chat).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_stale_alive_entry_without_a_room_is_dropped(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_on(pg.clone(), 9);
    let code = new_code();
    alive(&chat, &code, minutes_ago(30)).await?;

    ok(rooms::run_once(&chat).await)?;

    assert_eq!(alive_score(&chat, &code).await?, None);
    Ok(())
}
