use sqlx::PgPool;

use super::*;
use crate::modules::chat::conversations::create;
use crate::modules::chat::conversations::fixtures::{access_of, dm, group, post, seqs};
use crate::modules::chat::members::access;

#[test]
fn only_off_an_hour_a_day_and_a_week_are_timers() {
    for allowed in [None, Some(3_600), Some(86_400), Some(604_800)] {
        assert_eq!(checked(allowed), Ok(allowed));
    }
    for refused in [Some(0), Some(60), Some(-3_600), Some(2_592_000)] {
        assert_eq!(checked(refused), Err(ChatError::Invalid));
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn any_dm_member_sets_the_timer_and_a_system_row_says_so(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let row = set(&pool, &access_of(&pool, id, "202").await, Some(86_400))
        .await?
        .expect("a change");
    assert_eq!(
        (row.class.as_str(), row.sender_user.as_str()),
        ("system", "202")
    );
    assert_eq!(row.expires_at, None);
    let body: serde_json::Value =
        serde_json::from_slice(row.body.as_deref().expect("json")).expect("system json");
    assert_eq!(
        body,
        serde_json::json!({"kind": "ttl", "actor": "202", "ttl_s": 86_400})
    );
    assert_eq!(access_of(&pool, id, "101").await.ttl_s, Some(86_400));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_same_timer_twice_writes_one_row(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let access = access_of(&pool, id, "101").await;
    assert!(set(&pool, &access, Some(3_600)).await?.is_some());
    assert!(set(&pool, &access, Some(3_600)).await?.is_none());
    assert_eq!(seqs(&pool, id).await, vec![1]);
    let off = set(&pool, &access, None).await?.expect("a change");
    let body: serde_json::Value =
        serde_json::from_slice(off.body.as_deref().expect("json")).expect("system json");
    assert_eq!(body["ttl_s"], serde_json::Value::Null);
    assert_eq!(access_of(&pool, id, "101").await.ttl_s, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn in_a_group_only_admins_set_the_timer(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    assert_eq!(
        set(&pool, &access_of(&pool, id, "202").await, Some(3_600))
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    assert!(
        set(&pool, &access_of(&pool, id, "101").await, Some(3_600))
            .await?
            .is_some()
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn saved_messages_and_requests_take_no_timer(pool: PgPool) -> ChatResult<()> {
    let saved = create::saved(&pool, "101").await?.id;
    assert_eq!(
        set(&pool, &access_of(&pool, saved, "101").await, Some(3_600))
            .await
            .err(),
        Some(ChatError::Invalid)
    );
    let request = dm(&pool, "101", "202", MemberState::Invited).await;
    assert_eq!(
        set(&pool, &access_of(&pool, request, "202").await, Some(3_600))
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_timer_applies_only_to_posts_sent_after_it(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    let before = post(&pool, id, "101").await;
    set(&pool, &access_of(&pool, id, "101").await, Some(3_600)).await?;
    let after = post(&pool, id, "202").await;
    assert_eq!(before.expires_at, None);
    assert!(after.expires_at.is_some());
    let stored: Vec<Option<chrono::DateTime<chrono::Utc>>> = sqlx::query_scalar(
        "SELECT expires_at FROM chat.messages WHERE conversation_id = $1 ORDER BY seq",
    )
    .bind(id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(stored.iter().filter(|at| at.is_some()).count(), 1);
    let reloaded = access::load(&pool, id, "101").await?;
    assert_eq!(reloaded.last_seq, 3);
    Ok(())
}
