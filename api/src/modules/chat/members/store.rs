use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::members::roles;
use crate::modules::chat::model::{MemberRole, MemberState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Joining {
    pub user_id: String,
    pub role: MemberRole,
    pub state: MemberState,
    pub accepted: bool,
}

impl Joining {
    pub fn creator(user_id: &str, role: MemberRole) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role,
            state: MemberState::Active,
            accepted: true,
        }
    }

    pub fn placed(user_id: String, state: MemberState) -> Self {
        Self {
            user_id,
            role: MemberRole::Member,
            state,
            accepted: false,
        }
    }
}

pub struct Joined {
    pub user_id: String,
    pub state: MemberState,
}

pub async fn join_many(
    conn: &mut PgConnection,
    conversation: Uuid,
    joined_seq: i64,
    joined_epoch: i32,
    invited_by: &str,
    joining: &[Joining],
) -> ChatResult<Vec<Joined>> {
    let users: Vec<String> = joining.iter().map(|one| one.user_id.clone()).collect();
    let role_names: Vec<String> = joining
        .iter()
        .map(|one| roles::role_name(one.role).to_owned())
        .collect();
    let states: Vec<String> = joining
        .iter()
        .map(|one| roles::state_name(one.state).to_owned())
        .collect();
    let accepted: Vec<bool> = joining.iter().map(|one| one.accepted).collect();
    let rows = sqlx::query_file!(
        "queries/chat/members/join_many.sql",
        conversation,
        joined_seq,
        joined_epoch,
        invited_by,
        &users,
        &role_names,
        &states,
        &accepted,
    )
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(Joined {
                state: roles::state(&row.state)?,
                user_id: row.user_id,
            })
        })
        .collect()
}

pub async fn recount(conn: &mut PgConnection, conversation: Uuid) -> ChatResult<i32> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/conversations/recount.sql", conversation)
            .fetch_one(&mut *conn)
            .await?,
    )
}

pub async fn set_state(
    conn: &mut PgConnection,
    conversation: Uuid,
    user_id: &str,
    state: MemberState,
    from: &[MemberState],
) -> ChatResult<Option<MemberRole>> {
    let from: Vec<String> = from
        .iter()
        .map(|state| roles::state_name(*state).to_owned())
        .collect();
    sqlx::query_file_scalar!(
        "queries/chat/members/set_state.sql",
        conversation,
        user_id,
        roles::state_name(state),
        &from,
    )
    .fetch_optional(&mut *conn)
    .await?
    .map(|role| roles::role(&role))
    .transpose()
}

pub async fn activate(
    conn: &mut PgConnection,
    conversation: Uuid,
    user_id: &str,
    from: &[MemberState],
) -> ChatResult<bool> {
    let from: Vec<String> = from
        .iter()
        .map(|state| roles::state_name(*state).to_owned())
        .collect();
    Ok(sqlx::query_file_scalar!(
        "queries/chat/members/accept.sql",
        conversation,
        user_id,
        &from
    )
    .fetch_optional(&mut *conn)
    .await?
    .is_some())
}

pub async fn set_role(
    conn: &mut PgConnection,
    conversation: Uuid,
    user_id: &str,
    role: MemberRole,
) -> ChatResult<bool> {
    Ok(sqlx::query_file_scalar!(
        "queries/chat/members/set_role.sql",
        conversation,
        user_id,
        roles::role_name(role)
    )
    .fetch_optional(&mut *conn)
    .await?
    .is_some())
}

pub async fn standing(
    conn: &mut PgConnection,
    conversation: Uuid,
    user_id: &str,
) -> ChatResult<Option<(MemberRole, MemberState)>> {
    sqlx::query_file!("queries/chat/members/state_of.sql", conversation, user_id)
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| Ok((roles::role(&row.role)?, roles::state(&row.state)?)))
        .transpose()
}

pub async fn successor(
    conn: &mut PgConnection,
    conversation: Uuid,
    leaving: &str,
) -> ChatResult<Option<String>> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/members/successor.sql", conversation, leaving)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn live_total(conn: &mut PgConnection, conversation: Uuid) -> ChatResult<i64> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/members/live_total.sql", conversation)
            .fetch_one(&mut *conn)
            .await?,
    )
}
