use std::collections::HashMap;

use sqlx::PgPool;

use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::MemberState;
use crate::modules::chat::settings::DmPolicy;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatResult, ChatState, blocks};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Relation {
    pub follows: bool,
    pub contact: bool,
}

impl Relation {
    pub fn known(self) -> bool {
        self.follows || self.contact
    }
}

pub fn placement(relation: Relation, policy: DmPolicy) -> Option<MemberState> {
    match policy {
        DmPolicy::Everyone => Some(MemberState::Active),
        DmPolicy::Following => Some(known_or_invited(relation)),
        DmPolicy::Nobody => relation.contact.then_some(MemberState::Active),
    }
}

pub fn group_placement(relation: Relation, policy: DmPolicy) -> Option<MemberState> {
    match policy {
        DmPolicy::Everyone | DmPolicy::Following => Some(known_or_invited(relation)),
        DmPolicy::Nobody => relation.contact.then_some(MemberState::Active),
    }
}

fn known_or_invited(relation: Relation) -> MemberState {
    if relation.known() {
        MemberState::Active
    } else {
        MemberState::Invited
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

pub async fn relation(pg: &PgPool, user: &str, other: &str) -> ChatResult<Relation> {
    Ok(relations(pg, user, &[other.to_owned()])
        .await?
        .remove(other)
        .unwrap_or_default())
}

pub async fn policy(pg: &PgPool, user: &str) -> ChatResult<DmPolicy> {
    Ok(policies(pg, &[user.to_owned()])
        .await?
        .remove(user)
        .unwrap_or(DmPolicy::Following))
}

pub async fn landing(
    chat: &ChatState,
    sender: &str,
    recipient: &str,
    relation: Relation,
) -> ChatResult<Option<MemberState>> {
    if sender == recipient {
        return Ok(Some(MemberState::Active));
    }
    if blocks::blocked_between(chat, sender, recipient).await? {
        return Ok(None);
    }
    let policy = policy(&chat.pg, recipient).await?;
    Ok(placement(relation, policy))
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
            group_placement(relation, policy).map(|state| (candidate.clone(), state))
        })
        .collect())
}

pub async fn unblocked(
    chat: &ChatState,
    user: &str,
    wanted: Vec<String>,
) -> ChatResult<Vec<String>> {
    let blocked = blocks::blocked_among(chat, user, &wanted).await?;
    Ok(wanted
        .into_iter()
        .filter(|other| !blocked.contains(other))
        .collect())
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
