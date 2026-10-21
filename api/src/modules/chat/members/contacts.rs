use std::collections::HashMap;

use sqlx::PgPool;

use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::MemberState;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatResult, ChatState, blocks};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmPolicy {
    Everyone,
    Following,
    Nobody,
}

impl DmPolicy {
    fn parse(raw: &str) -> Self {
        match raw {
            "everyone" => DmPolicy::Everyone,
            "nobody" => DmPolicy::Nobody,
            _ => DmPolicy::Following,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Relation {
    pub follows: bool,
    pub contact: bool,
}

pub fn placement(relation: Relation, policy: DmPolicy) -> Option<MemberState> {
    let known = relation.follows || relation.contact;
    match policy {
        DmPolicy::Everyone if known => Some(MemberState::Active),
        DmPolicy::Everyone => Some(MemberState::Invited),
        DmPolicy::Following if known => Some(MemberState::Active),
        DmPolicy::Nobody if relation.contact => Some(MemberState::Active),
        DmPolicy::Following | DmPolicy::Nobody => None,
    }
}

pub async fn policies(pg: &PgPool, users: &[String]) -> ChatResult<HashMap<String, DmPolicy>> {
    let rows = sqlx::query_file!("queries/chat/conversations/policies.sql", users)
        .fetch_all(pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.user_id, DmPolicy::parse(&row.dm_policy)))
        .collect())
}

pub async fn relations(
    pg: &PgPool,
    user: &str,
    candidates: &[String],
) -> ChatResult<HashMap<String, Relation>> {
    let rows = sqlx::query_file!("queries/chat/conversations/contacts.sql", user, candidates)
        .fetch_all(pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let relation = Relation {
                follows: row.follows,
                contact: row.contact,
            };
            (row.user_id, relation)
        })
        .collect())
}

pub async fn placements(
    pg: &PgPool,
    user: &str,
    candidates: &[String],
) -> ChatResult<Vec<(String, MemberState)>> {
    let relations = relations(pg, user, candidates).await?;
    let policies = policies(pg, candidates).await?;
    Ok(candidates
        .iter()
        .filter_map(|candidate| {
            let relation = relations.get(candidate).copied().unwrap_or_default();
            let policy = policies
                .get(candidate)
                .copied()
                .unwrap_or(DmPolicy::Following);
            placement(relation, policy).map(|state| (candidate.clone(), state))
        })
        .collect())
}

pub async fn unblocked(
    chat: &ChatState,
    user: &str,
    wanted: Vec<String>,
) -> ChatResult<Vec<String>> {
    let mut open = Vec::with_capacity(wanted.len());
    for other in wanted {
        if !blocks::blocked_between(chat, user, &other).await? {
            open.push(other);
        }
    }
    Ok(open)
}

pub async fn admit_invites(
    chat: &ChatState,
    user: &str,
    placed: &[(String, MemberState)],
) -> ChatResult<()> {
    if !placed
        .iter()
        .any(|(_, state)| *state == MemberState::Invited)
    {
        return Ok(());
    }
    users::check(chat, user, Scope::DmNew).await?;
    limits::admit(chat, Bucket::Invite, user).await
}

#[cfg(test)]
#[path = "../tests/contacts_tests.rs"]
mod tests;
