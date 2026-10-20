use sqlx::PgPool;

use super::*;
use crate::modules::chat::conversations::fixtures::{
    access_of, dm, dm_policy, follow, group, post, seqs,
};
use crate::modules::chat::conversations::view;

async fn members_of(pg: &PgPool, conversation: Uuid) -> Vec<(String, String, String, bool)> {
    sqlx::query_as(
        "SELECT user_id, role, state, accepted FROM chat.members
         WHERE conversation_id = $1 ORDER BY user_id",
    )
    .bind(conversation)
    .fetch_all(pg)
    .await
    .expect("members")
}

#[sqlx::test(migrations = "./migrations")]
async fn a_dm_is_one_conversation_from_either_side(pool: PgPool) -> ChatResult<()> {
    follow(&pool, "202", "101").await;
    let first = plan_direct(&pool, "101", "202").await?;
    assert_eq!(
        first,
        DirectPlan::New {
            peer: "202".to_owned(),
            peer_state: MemberState::Active
        }
    );
    let created = direct(&pool, "101", first).await?;
    let again = plan_direct(&pool, "202", "101").await?;
    assert_eq!(again, DirectPlan::Existing(created.id));
    assert_eq!(direct(&pool, "202", again).await?.id, created.id);
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chat.conversations WHERE kind = 'direct'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(total, 1);
    assert_eq!(members_of(&pool, created.id).await.len(), 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn two_racing_creates_of_one_dm_end_in_one_row(pool: PgPool) -> ChatResult<()> {
    let plan = DirectPlan::New {
        peer: "202".to_owned(),
        peer_state: MemberState::Active,
    };
    let (left, right) = tokio::join!(
        direct(&pool, "101", plan.clone()),
        direct(&pool, "101", plan.clone())
    );
    assert_eq!(left?.id, right?.id);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_recipient_policy_decides_where_a_stranger_lands(pool: PgPool) -> ChatResult<()> {
    assert_eq!(
        plan_direct(&pool, "101", "202").await,
        Err(ChatError::Unavailable)
    );
    dm_policy(&pool, "202", "everyone").await;
    let plan = plan_direct(&pool, "101", "202").await?;
    assert!(plan.is_request());
    let id = direct(&pool, "101", plan).await?.id;
    let members = members_of(&pool, id).await;
    assert_eq!(
        members,
        vec![
            ("101".into(), "member".into(), "active".into(), true),
            ("202".into(), "member".into(), "invited".into(), false),
        ]
    );
    dm_policy(&pool, "303", "nobody").await;
    follow(&pool, "303", "101").await;
    assert_eq!(
        plan_direct(&pool, "101", "303").await,
        Err(ChatError::Unavailable)
    );
    follow(&pool, "404", "101").await;
    assert!(!plan_direct(&pool, "101", "404").await?.is_request());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn sharing_an_accepted_group_makes_a_contact(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    dm_policy(&pool, "101", "nobody").await;
    assert_eq!(
        plan_direct(&pool, "202", "101").await,
        Err(ChatError::Unavailable)
    );
    post(&pool, id, "202").await;
    assert_eq!(
        plan_direct(&pool, "202", "101").await,
        Ok(DirectPlan::New {
            peer: "101".to_owned(),
            peer_state: MemberState::Active
        })
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_declined_dm_reopens_only_for_the_one_who_comes_back(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Invited).await;
    sqlx::query("UPDATE chat.members SET state = 'left' WHERE user_id = '202'")
        .execute(&pool)
        .await?;
    direct(&pool, "101", plan_direct(&pool, "101", "202").await?).await?;
    assert_eq!(access_of(&pool, id, "202").await.state, MemberState::Left);
    direct(&pool, "202", plan_direct(&pool, "202", "101").await?).await?;
    let back = access_of(&pool, id, "202").await;
    assert_eq!((back.state, back.accepted), (MemberState::Active, true));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn saved_messages_are_one_per_user(pool: PgPool) -> ChatResult<()> {
    let first = saved(&pool, "101").await?.id;
    assert_eq!(saved(&pool, "101").await?.id, first);
    assert_ne!(saved(&pool, "202").await?.id, first);
    let view = view::one(&pool, first, "101").await?;
    assert_eq!(view.kind, "self");
    assert_eq!(view.me.role, "owner");
    assert_eq!(view.peer, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_group_starts_with_its_owner_and_a_created_row(pool: PgPool) -> ChatResult<()> {
    let created = super::group(
        &pool,
        "101",
        vec![
            ("202".to_owned(), MemberState::Active),
            ("303".to_owned(), MemberState::Invited),
        ],
        Some((vec![1; 64], 0)),
    )
    .await?;
    assert_eq!(seqs(&pool, created.id).await, vec![1]);
    assert_eq!(created.rows[0].class, "system");
    assert_eq!(created.recipients, vec!["101", "202", "303"]);
    let view = view::one(&pool, created.id, "101").await?;
    assert_eq!(view.member_count, 2);
    assert_eq!(view.me.role, "owner");
    assert_eq!(view.meta_ct, Some(vec![1; 64]));
    let invited = view::one(&pool, created.id, "303").await?;
    assert_eq!(invited.me.state, "invited");
    assert_eq!(invited.me.invited_by.as_deref(), Some("101"));
    Ok(())
}

#[test]
fn a_group_holds_at_most_200_people() {
    let others: Vec<String> = (0..199).map(|n| format!("{}", 1_000 + n)).collect();
    assert_eq!(group_members("1", &others, false).map(|v| v.len()), Ok(199));
    let mut crowd = others.clone();
    crowd.push("9999".to_owned());
    assert_eq!(group_members("1", &crowd, false), Err(ChatError::Invalid));
    let mut repeats = others;
    repeats.push("1000".to_owned());
    repeats.push("1".to_owned());
    assert_eq!(
        group_members("1", &repeats, false).map(|v| v.len()),
        Ok(199)
    );
}

#[test]
fn a_dm_names_exactly_one_other_valid_user() {
    assert_eq!(
        single_peer("1", &["2".to_owned()], false),
        Ok("2".to_owned())
    );
    assert_eq!(
        single_peer("1", &["1".to_owned()], false),
        Err(ChatError::Invalid)
    );
    assert_eq!(single_peer("1", &[], false), Err(ChatError::Invalid));
    assert_eq!(
        single_peer("1", &["2".to_owned(), "3".to_owned()], false),
        Err(ChatError::Invalid)
    );
    assert_eq!(
        single_peer("1", &["x".to_owned()], false),
        Err(ChatError::Invalid)
    );
}

#[test]
fn group_meta_comes_whole_and_small() {
    assert_eq!(meta(None, None), Ok(None));
    assert_eq!(meta(Some(vec![1]), Some(0)), Ok(Some((vec![1], 0))));
    assert_eq!(meta(Some(vec![1]), None), Err(ChatError::Invalid));
    assert_eq!(meta(None, Some(0)), Err(ChatError::Invalid));
    assert_eq!(meta(Some(vec![]), Some(0)), Err(ChatError::Invalid));
    assert_eq!(
        meta(Some(vec![1; META_MAX + 1]), Some(0)),
        Err(ChatError::Invalid)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_conversation_cap_stops_new_chats(pool: PgPool) -> ChatResult<()> {
    sqlx::query(
        "INSERT INTO chat.members (conversation_id, user_id, state)
         SELECT gen_random_uuid(), '101', 'active' FROM generate_series(1, 1000)",
    )
    .execute(&pool)
    .await?;
    assert_eq!(under_cap(&pool, "101").await, Err(ChatError::RequestLimit));
    assert_eq!(under_cap(&pool, "202").await, Ok(()));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_list_pages_by_cursor_and_skips_left_chats(pool: PgPool) -> ChatResult<()> {
    let mut made = Vec::new();
    for peer in ["202", "303", "404"] {
        made.push(dm(&pool, "101", peer, MemberState::Active).await);
    }
    made.sort();
    let left = group(&pool, "505", &[("101", MemberState::Active)]).await;
    sqlx::query(
        "UPDATE chat.members SET state = 'left' WHERE conversation_id = $1 AND user_id = '101'",
    )
    .bind(left)
    .execute(&pool)
    .await?;
    let first = view::fit(
        view::rows_after(&pool, "101", Uuid::nil(), 3).await?,
        "101",
        2,
        128 * 1024,
    );
    assert_eq!(
        first.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        made[..2]
    );
    let cursor: Uuid = first
        .next_cursor
        .as_deref()
        .expect("a next page")
        .parse()
        .expect("a uuid cursor");
    let rest = view::fit(
        view::rows_after(&pool, "101", cursor, 3).await?,
        "101",
        2,
        128 * 1024,
    );
    assert_eq!(
        rest.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        made[2..]
    );
    assert_eq!(rest.next_cursor, None);
    assert!(rest.items[0].peer.is_some());
    let tight = view::fit(
        view::rows_after(&pool, "101", Uuid::nil(), 4).await?,
        "101",
        200,
        700,
    );
    assert!(tight.items.len() < 3);
    assert!(serde_json::to_vec(&tight).expect("json").len() <= 700);
    assert!(tight.next_cursor.is_some());
    Ok(())
}
