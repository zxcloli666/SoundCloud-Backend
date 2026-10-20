use chat_wire::msg::Class;
use sqlx::PgPool;

use super::*;
use crate::modules::chat::conversations::fixtures::{access_of, dm, group, outgoing, post, send};

async fn body_of(pool: &PgPool, conversation: uuid::Uuid, seq: i64) -> Option<Vec<u8>> {
    sqlx::query_scalar("SELECT body FROM chat.messages WHERE conversation_id = $1 AND seq = $2")
        .bind(conversation)
        .bind(seq)
        .fetch_one(pool)
        .await
        .expect("a row")
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_for_everyone_wipes_the_post_and_its_edits(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let target = post(&pool, id, "101").await;
    let edit = send(
        &pool,
        id,
        "101",
        &outgoing(Class::Edit, 0, Some(target.seq)),
    )
    .await?;
    let reaction = send(
        &pool,
        id,
        "202",
        &outgoing(Class::React, 0, Some(target.seq)),
    )
    .await?;
    let rows = for_everyone(&pool, &access_of(&pool, id, "101").await, target.seq).await?;
    let touched: Vec<(i64, String)> = rows
        .iter()
        .map(|row| (row.seq, row.class.clone()))
        .collect();
    assert_eq!(
        touched,
        vec![
            (target.seq, "post".to_owned()),
            (edit.seq, "edit".to_owned()),
            (reaction.seq + 1, "system".to_owned())
        ]
    );
    assert_eq!(body_of(&pool, id, target.seq).await, None);
    assert_eq!(body_of(&pool, id, edit.seq).await, None);
    assert!(body_of(&pool, id, reaction.seq).await.is_some());
    let marker = rows.last().expect("a system row");
    assert_eq!(marker.target_seq, Some(target.seq));
    let body: serde_json::Value =
        serde_json::from_slice(marker.body.as_deref().expect("json")).expect("system json");
    assert_eq!(body, serde_json::json!({"kind": "deleted", "actor": "101"}));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_delete_changes_nothing(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let target = post(&pool, id, "101").await;
    let access = access_of(&pool, id, "101").await;
    for_everyone(&pool, &access, target.seq).await?;
    assert!(for_everyone(&pool, &access, target.seq).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn only_the_author_or_a_group_admin_deletes(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let target = post(&pool, id, "101").await;
    assert_eq!(
        for_everyone(&pool, &access_of(&pool, id, "202").await, target.seq)
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    let room = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    let theirs = post(&pool, room, "202").await;
    assert_eq!(
        for_everyone(&pool, &access_of(&pool, room, "303").await, theirs.seq)
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    assert_eq!(
        for_everyone(&pool, &access_of(&pool, room, "101").await, theirs.seq)
            .await?
            .len(),
        2
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn system_rows_and_missing_rows_are_not_deletable(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    let access = access_of(&pool, id, "101").await;
    assert_eq!(
        for_everyone(&pool, &access, 1).await.err(),
        Some(ChatError::Invalid)
    );
    assert_eq!(
        for_everyone(&pool, &access, 99).await.err(),
        Some(ChatError::NotFound)
    );
    Ok(())
}
