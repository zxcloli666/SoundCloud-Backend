use chat_wire::msg::Class;
use chrono::Duration;
use futures::future::join_all;
use sqlx::PgPool;

use super::*;
use crate::modules::chat::conversations::fixtures::{
    access_of, dm, exec, group, outgoing, post, send, seqs,
};
use crate::modules::chat::messages::gate;
use crate::modules::chat::model::MemberState;

#[sqlx::test(migrations = "./migrations")]
async fn a_resent_message_keeps_its_seq_and_stays_one_row(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let out = outgoing(Class::Post, 0, None);
    let first = send(&pool, id, "101", &out).await?;
    let again = send(&pool, id, "101", &out).await?;
    assert_eq!(first.seq, again.seq);
    assert_eq!(first.created_at, again.created_at);
    assert_eq!(seqs(&pool, id).await, vec![1]);
    let next = post(&pool, id, "101").await;
    assert_eq!(next.seq, 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_resend_after_a_rekey_still_finds_the_stored_row(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let out = outgoing(Class::Post, 0, None);
    let first = send(&pool, id, "101", &out).await?;
    exec(
        &pool,
        "UPDATE chat.conversations SET rekey = true WHERE id = $1",
        id,
    )
    .await;
    assert_eq!(send(&pool, id, "101", &out).await?.seq, first.seq);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn someone_else_cannot_claim_a_client_id(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let out = outgoing(Class::Post, 0, None);
    send(&pool, id, "101", &out).await?;
    assert_eq!(
        send(&pool, id, "202", &out).await.err(),
        Some(ChatError::Invalid)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_sends_keep_seq_gapless(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    let outs: Vec<(&str, Outgoing)> = (0..30)
        .map(|n| (["101", "202", "303"][n % 3], outgoing(Class::Post, 0, None)))
        .collect();
    let resend = outs[4].clone();
    let sends = outs
        .iter()
        .chain(std::iter::once(&resend))
        .map(|(user, out)| send(&pool, id, user, out));
    for result in join_all(sends).await {
        result?;
    }
    assert_eq!(seqs(&pool, id).await, (1..=31).collect::<Vec<i64>>());
    assert_eq!(access_of(&pool, id, "101").await.last_seq, 31);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_pending_rekey_turns_the_epoch_once(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    post(&pool, id, "101").await;
    exec(
        &pool,
        "UPDATE chat.conversations SET rekey = true WHERE id = $1",
        id,
    )
    .await;
    let stale = send(&pool, id, "101", &outgoing(Class::Post, 0, None)).await;
    assert_eq!(stale.err(), Some(ChatError::EpochStale));
    assert_eq!(access_of(&pool, id, "101").await.key_epoch, 1);
    assert_eq!(
        send(&pool, id, "202", &outgoing(Class::Post, 0, None))
            .await
            .err(),
        Some(ChatError::EpochStale)
    );
    assert_eq!(access_of(&pool, id, "101").await.key_epoch, 1);
    let fresh = send(&pool, id, "101", &outgoing(Class::Post, 1, None)).await?;
    assert_eq!((fresh.seq, fresh.epoch), (2, 1));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_epoch_turns_after_10k_messages_or_30_days(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    exec(
        &pool,
        "UPDATE chat.conversations SET epoch_messages = 10000 WHERE id = $1",
        id,
    )
    .await;
    assert_eq!(
        send(&pool, id, "101", &outgoing(Class::Post, 0, None))
            .await
            .err(),
        Some(ChatError::EpochStale)
    );
    assert_eq!(access_of(&pool, id, "101").await.key_epoch, 1);
    send(&pool, id, "101", &outgoing(Class::Post, 1, None)).await?;
    exec(
        &pool,
        "UPDATE chat.conversations SET epoch_started_at = now() - interval '31 days' WHERE id = $1",
        id,
    )
    .await;
    assert_eq!(
        send(&pool, id, "101", &outgoing(Class::Post, 1, None))
            .await
            .err(),
        Some(ChatError::EpochStale)
    );
    assert_eq!(access_of(&pool, id, "101").await.key_epoch, 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_future_epoch_is_stale_and_turns_nothing(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    assert_eq!(
        send(&pool, id, "101", &outgoing(Class::Post, 5, None))
            .await
            .err(),
        Some(ChatError::EpochStale)
    );
    assert_eq!(access_of(&pool, id, "101").await.key_epoch, 0);
    assert_eq!(seqs(&pool, id).await, Vec::<i64>::new());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn posts_take_the_timer_from_the_send_update(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let before = post(&pool, id, "101").await;
    assert_eq!(before.expires_at, None);
    exec(
        &pool,
        "UPDATE chat.conversations SET ttl_s = 3600 WHERE id = $1",
        id,
    )
    .await;
    let after = post(&pool, id, "101").await;
    assert_eq!(
        after.expires_at,
        Some(after.created_at + Duration::hours(1))
    );
    let stored: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT expires_at FROM chat.messages WHERE conversation_id = $1 AND seq = $2",
    )
    .bind(id)
    .bind(after.seq)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        stored.map(|at| at.timestamp_millis()),
        after.expires_at.map(|at| at.timestamp_millis())
    );
    let reaction = send(
        &pool,
        id,
        "202",
        &outgoing(Class::React, 0, Some(after.seq)),
    )
    .await?;
    assert_eq!(reaction.expires_at, after.expires_at);
    let old_reaction = send(
        &pool,
        id,
        "202",
        &outgoing(Class::React, 0, Some(before.seq)),
    )
    .await?;
    assert_eq!(old_reaction.expires_at, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_expired_target_refuses_edits_reactions_and_pins(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let target = post(&pool, id, "101").await;
    exec(
        &pool,
        "UPDATE chat.messages SET expires_at = now() - interval '1 second' WHERE conversation_id = $1",
        id,
    )
    .await;
    for class in [Class::Edit, Class::React, Class::Pin] {
        assert_eq!(
            send(&pool, id, "101", &outgoing(class, 0, Some(target.seq)))
                .await
                .err(),
            Some(ChatError::NotFound),
            "{class:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn edits_belong_to_the_author(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let target = post(&pool, id, "101").await;
    assert_eq!(
        send(
            &pool,
            id,
            "202",
            &outgoing(Class::Edit, 0, Some(target.seq))
        )
        .await
        .err(),
        Some(ChatError::NotMember)
    );
    send(
        &pool,
        id,
        "101",
        &outgoing(Class::Edit, 0, Some(target.seq)),
    )
    .await?;
    exec(
        &pool,
        "UPDATE chat.messages SET created_at = now() - interval '49 hours' WHERE conversation_id = $1",
        id,
    )
    .await;
    assert_eq!(
        send(
            &pool,
            id,
            "101",
            &outgoing(Class::Edit, 0, Some(target.seq))
        )
        .await
        .err(),
        Some(ChatError::Invalid)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_stranger_is_not_a_member(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    assert_eq!(
        send(&pool, id, "303", &outgoing(Class::Post, 0, None))
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn group_meta_rides_the_send_update(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    let mut meta = outgoing(Class::Meta, 0, None);
    meta.body = vec![9; 300];
    assert_eq!(
        send(&pool, id, "202", &meta).await.err(),
        Some(ChatError::NotMember)
    );
    let row = send(&pool, id, "101", &meta).await?;
    let stored: (Option<Vec<u8>>, Option<i32>) =
        sqlx::query_as("SELECT meta_ct, meta_epoch FROM chat.conversations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, (Some(vec![9; 300]), Some(0)));
    assert_eq!(row.class, "meta");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_first_post_accepts_the_membership(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    assert!(!access_of(&pool, id, "202").await.accepted);
    post(&pool, id, "202").await;
    assert!(access_of(&pool, id, "202").await.accepted);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_request_holds_three_posts_until_accepted(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Invited).await;
    for _ in 0..3 {
        let access = access_of(&pool, id, "101").await;
        gate::peer_open(&access)?;
        gate::request_room(&pool, &access, Class::Post).await?;
        post(&pool, id, "101").await;
    }
    let access = access_of(&pool, id, "101").await;
    assert_eq!(
        gate::request_room(&pool, &access, Class::Post).await,
        Err(ChatError::RequestLimit)
    );
    assert_eq!(
        gate::request_room(&pool, &access, Class::React).await,
        Ok(())
    );
    exec(
        &pool,
        "UPDATE chat.members SET state = 'left' WHERE conversation_id = $1 AND user_id = '202'",
        id,
    )
    .await;
    let declined = access_of(&pool, id, "101").await;
    assert_eq!(gate::peer_open(&declined), Err(ChatError::Unavailable));
    Ok(())
}
