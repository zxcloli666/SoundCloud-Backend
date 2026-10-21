use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::modules::chat::conversations::fixtures::{access_of, dm, exec, group, seqs};

async fn rekey_of(pool: &PgPool, conversation: Uuid) -> bool {
    sqlx::query_scalar("SELECT rekey FROM chat.conversations WHERE id = $1")
        .bind(conversation)
        .fetch_one(pool)
        .await
        .expect("a conversation")
}

async fn state_of(pool: &PgPool, conversation: Uuid, user: &str) -> (String, String) {
    sqlx::query_as(
        "SELECT role, state FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation)
    .bind(user)
    .fetch_one(pool)
    .await
    .expect("a member")
}

fn kinds(change: &Change) -> Vec<String> {
    change
        .rows
        .iter()
        .map(|row| {
            let body: serde_json::Value =
                serde_json::from_slice(row.body.as_deref().unwrap_or_default())
                    .expect("system json");
            body["kind"].as_str().unwrap_or_default().to_owned()
        })
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn a_kick_rekeys_and_tells_everyone(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    let change = remove(&pool, &access_of(&pool, id, "101").await, "202", false).await?;
    assert_eq!(kinds(&change), vec!["kicked"]);
    assert!(change.recipients.contains(&"202".to_owned()));
    assert!(rekey_of(&pool, id).await);
    assert_eq!(state_of(&pool, id, "202").await.1, "kicked");
    assert_eq!(
        access_of(&pool, id, "101").await.recipients,
        vec!["101", "303"]
    );
    let again = remove(&pool, &access_of(&pool, id, "101").await, "202", false).await?;
    assert!(again.rows.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn admins_cannot_remove_admins_or_the_owner(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    exec(
        &pool,
        "UPDATE chat.members SET role = 'admin' WHERE conversation_id = $1 AND user_id IN ('202', '303')",
        id,
    )
    .await;
    let admin = access_of(&pool, id, "202").await;
    assert_eq!(
        remove(&pool, &admin, "303", false).await.err(),
        Some(ChatError::NotMember)
    );
    assert_eq!(
        remove(&pool, &admin, "101", true).await.err(),
        Some(ChatError::NotMember)
    );
    let member = group(&pool, "101", &[("404", MemberState::Active)]).await;
    assert_eq!(
        remove(&pool, &access_of(&pool, member, "404").await, "101", false)
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_ban_keeps_the_user_out_of_later_adds(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    let owner = access_of(&pool, id, "101").await;
    let change = remove(&pool, &owner, "202", true).await?;
    assert_eq!(kinds(&change), vec!["banned"]);
    let owner = access_of(&pool, id, "101").await;
    let readd = add(&pool, &owner, vec![("202".to_owned(), MemberState::Active)]).await?;
    assert!(readd.rows.is_empty());
    assert_eq!(state_of(&pool, id, "202").await.1, "banned");
    unban(&pool, &owner, "202").await?;
    let back = add(&pool, &owner, vec![("202".to_owned(), MemberState::Active)]).await?;
    assert_eq!(back.added, vec!["202"]);
    assert_eq!(state_of(&pool, id, "202").await.1, "active");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_owner_leaving_hands_the_group_over(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    exec(
        &pool,
        "UPDATE chat.members SET role = 'admin' WHERE conversation_id = $1 AND user_id = '303'",
        id,
    )
    .await;
    let change = leave(&pool, &access_of(&pool, id, "101").await).await?;
    assert_eq!(kinds(&change), vec!["role", "left"]);
    assert_eq!(state_of(&pool, id, "303").await.0, "owner");
    assert_eq!(state_of(&pool, id, "101").await.1, "left");
    assert!(rekey_of(&pool, id).await);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_last_member_out_deletes_the_group_and_its_keys(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Invited)]).await;
    sqlx::query(
        "INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version)
         VALUES ($1, 0, '\\x00', 1)",
    )
    .bind(id)
    .execute(&pool)
    .await?;
    let change = leave(&pool, &access_of(&pool, id, "101").await).await?;
    assert!(change.deleted);
    let (deleted, keys): (bool, i64) = sqlx::query_as(
        "SELECT deleted_at IS NOT NULL,
                (SELECT count(*) FROM chat.conversation_keys WHERE conversation_id = $1)
         FROM chat.conversations WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!((deleted, keys), (true, 0));
    assert_eq!(
        crate::modules::chat::members::access::load(&pool, id, "202")
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_dm_cannot_be_left_like_a_group(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Active).await;
    assert_eq!(
        leave(&pool, &access_of(&pool, id, "101").await).await.err(),
        Some(ChatError::Invalid)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_invite_is_accepted_or_declined_once(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Invited), ("303", MemberState::Invited)],
    )
    .await;
    let joined = accept(&pool, &access_of(&pool, id, "202").await).await?;
    assert_eq!(kinds(&joined), vec!["joined"]);
    assert_eq!(joined.requests, vec!["202".to_owned()]);
    assert_eq!(access_of(&pool, id, "202").await.state, MemberState::Active);
    assert!(
        accept(&pool, &access_of(&pool, id, "202").await)
            .await?
            .rows
            .is_empty()
    );
    let declined = decline(&pool, &access_of(&pool, id, "303").await).await?;
    assert!(declined.rows.is_empty());
    assert_eq!(declined.requests, vec!["303".to_owned()]);
    assert_eq!(state_of(&pool, id, "303").await.1, "left");
    let members: i32 =
        sqlx::query_scalar("SELECT member_count FROM chat.conversations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(members, 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_dm_request_is_accepted_without_a_system_row(pool: PgPool) -> ChatResult<()> {
    let id = dm(&pool, "101", "202", MemberState::Invited).await;
    let accepted = accept(&pool, &access_of(&pool, id, "202").await).await?;
    assert!(accepted.rows.is_empty());
    let peer = access_of(&pool, id, "202").await;
    assert_eq!((peer.state, peer.accepted), (MemberState::Active, true));
    assert_eq!(seqs(&pool, id).await, Vec::<i64>::new());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn adding_respects_the_200_cap_and_hidden_history(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[]).await;
    exec(
        &pool,
        "UPDATE chat.conversations SET settings = '{\"history_visible\": false}', key_epoch = 3 WHERE id = $1",
        id,
    )
    .await;
    let owner = access_of(&pool, id, "101").await;
    let change = add(
        &pool,
        &owner,
        vec![
            ("202".to_owned(), MemberState::Active),
            ("303".to_owned(), MemberState::Invited),
        ],
    )
    .await?;
    assert_eq!(kinds(&change), vec!["added"]);
    assert_eq!(change.requests, vec!["303".to_owned()]);
    assert!(rekey_of(&pool, id).await);
    let (joined_seq, joined_epoch): (i64, i32) = sqlx::query_as(
        "SELECT joined_seq, joined_epoch FROM chat.members WHERE conversation_id = $1 AND user_id = '202'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!((joined_seq, joined_epoch), (1, 4));
    let newcomer = access_of(&pool, id, "202").await;
    assert_eq!(newcomer.history_floor(), 1);
    let crowd: Vec<(String, MemberState)> = (0..198)
        .map(|n| (format!("{}", 5_000 + n), MemberState::Active))
        .collect();
    let owner = access_of(&pool, id, "101").await;
    assert_eq!(
        add(&pool, &owner, crowd).await.err(),
        Some(ChatError::Invalid)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn only_the_owner_changes_roles_and_can_hand_over(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    let owner = access_of(&pool, id, "101").await;
    let change = set_role(&pool, &owner, "202", MemberRole::Admin).await?;
    assert_eq!(kinds(&change), vec!["role"]);
    let admin = access_of(&pool, id, "202").await;
    assert_eq!(
        set_role(&pool, &admin, "303", MemberRole::Admin)
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    set_role(&pool, &owner, "303", MemberRole::Owner).await?;
    assert_eq!(state_of(&pool, id, "303").await.0, "owner");
    assert_eq!(state_of(&pool, id, "101").await.0, "admin");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_add_joins_at_the_seq_of_its_own_commit(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[]).await;
    exec(
        &pool,
        "UPDATE chat.conversations SET settings = '{\"history_visible\": false}' WHERE id = $1",
        id,
    )
    .await;
    let stale = access_of(&pool, id, "101").await;
    exec(
        &pool,
        "UPDATE chat.conversations SET last_seq = 7, key_epoch = 2 WHERE id = $1",
        id,
    )
    .await;
    add(&pool, &stale, vec![("202".to_owned(), MemberState::Active)]).await?;
    let (joined_seq, joined_epoch): (i64, i32) = sqlx::query_as(
        "SELECT joined_seq, joined_epoch FROM chat.members WHERE conversation_id = $1 AND user_id = '202'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!((joined_seq, joined_epoch), (7, 3));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn racing_adds_never_pass_the_cap(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[]).await;
    let owner = access_of(&pool, id, "101").await;
    let batch = |from: u32| -> Vec<(String, MemberState)> {
        (from..from + 100)
            .map(|n| (n.to_string(), MemberState::Active))
            .collect()
    };
    let (left, right) = tokio::join!(
        add(&pool, &owner, batch(5_000)),
        add(&pool, &owner, batch(6_000))
    );
    let passed = [left.is_ok(), right.is_ok()];
    assert_eq!(passed.iter().filter(|ok| **ok).count(), 1);
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.members WHERE conversation_id = $1 AND state = 'active'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(live, 101);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn two_handovers_at_once_leave_one_owner(pool: PgPool) -> ChatResult<()> {
    let id = group(
        &pool,
        "101",
        &[("202", MemberState::Active), ("303", MemberState::Active)],
    )
    .await;
    let owner = access_of(&pool, id, "101").await;
    let (left, right) = tokio::join!(
        set_role(&pool, &owner, "202", MemberRole::Owner),
        set_role(&pool, &owner, "303", MemberRole::Owner)
    );
    assert_eq!(u8::from(left.is_ok()) + u8::from(right.is_ok()), 1);
    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.members WHERE conversation_id = $1 AND role = 'owner'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(owners, 1);
    assert_eq!(state_of(&pool, id, "101").await.0, "admin");
    Ok(())
}
