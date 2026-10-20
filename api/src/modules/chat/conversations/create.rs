use std::collections::BTreeSet;

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::conversations::CreateConversation;
use crate::modules::chat::conversations::keys::{dm_key, saved_key, valid_user_id};
use crate::modules::chat::members::contacts;
use crate::modules::chat::members::store::{self as members, Joining};
use crate::modules::chat::messages::system::{self, SystemEvent};
use crate::modules::chat::model::{
    ConversationKind, GROUP_MEMBERS_MAX, MemberRole, MemberState, MessageRow,
};
use crate::modules::chat::{ChatError, ChatResult};

pub const CONVERSATIONS_MAX: i64 = 1_000;
pub const META_MAX: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectPlan {
    Existing(Uuid),
    New {
        peer: String,
        peer_state: MemberState,
    },
}

impl DirectPlan {
    pub fn is_request(&self) -> bool {
        matches!(
            self,
            DirectPlan::New {
                peer_state: MemberState::Invited,
                ..
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wanted {
    Saved,
    Direct(String),
    Group {
        members: Vec<String>,
        meta: Option<(Vec<u8>, i32)>,
    },
}

pub fn wanted(user: &str, input: CreateConversation, synthetic: bool) -> ChatResult<Wanted> {
    match input.kind {
        ConversationKind::Saved => Ok(Wanted::Saved),
        ConversationKind::Direct => {
            single_peer(user, &input.members, synthetic).map(Wanted::Direct)
        }
        ConversationKind::Group => Ok(Wanted::Group {
            members: group_members(user, &input.members, synthetic)?,
            meta: meta(input.meta_ct, input.meta_epoch)?,
        }),
        ConversationKind::Room => Err(ChatError::Invalid),
    }
}

pub struct Created {
    pub id: Uuid,
    pub rows: Vec<MessageRow>,
    pub recipients: Vec<String>,
}

pub fn single_peer(user: &str, members: &[String], synthetic: bool) -> ChatResult<String> {
    match members {
        [peer] if peer != user && valid_user_id(peer, synthetic) => Ok(peer.clone()),
        _ => Err(ChatError::Invalid),
    }
}

pub fn group_members(user: &str, members: &[String], synthetic: bool) -> ChatResult<Vec<String>> {
    let unique: BTreeSet<&String> = members.iter().filter(|member| *member != user).collect();
    if unique.len() + 1 > GROUP_MEMBERS_MAX
        || unique
            .iter()
            .any(|member| !valid_user_id(member, synthetic))
    {
        return Err(ChatError::Invalid);
    }
    Ok(unique.into_iter().cloned().collect())
}

pub fn meta(
    meta_ct: Option<Vec<u8>>,
    meta_epoch: Option<i32>,
) -> ChatResult<Option<(Vec<u8>, i32)>> {
    match (meta_ct, meta_epoch) {
        (None, None) => Ok(None),
        (Some(ct), Some(epoch)) if !ct.is_empty() && ct.len() <= META_MAX && epoch >= 0 => {
            Ok(Some((ct, epoch)))
        }
        _ => Err(ChatError::Invalid),
    }
}

pub async fn under_cap(pg: &PgPool, user: &str) -> ChatResult<()> {
    let live = sqlx::query_file_scalar!("queries/chat/conversations/live_count.sql", user)
        .fetch_one(pg)
        .await?;
    if live >= CONVERSATIONS_MAX {
        return Err(ChatError::RequestLimit);
    }
    Ok(())
}

async fn find_dm(pg: &PgPool, key: &str) -> ChatResult<Option<Uuid>> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/conversations/find_dm.sql", key)
            .fetch_optional(pg)
            .await?,
    )
}

pub async fn plan_direct(pg: &PgPool, user: &str, peer: &str) -> ChatResult<DirectPlan> {
    if let Some(id) = find_dm(pg, &dm_key(user, peer)).await? {
        return Ok(DirectPlan::Existing(id));
    }
    let placed = contacts::placements(pg, user, &[peer.to_owned()])
        .await?
        .into_iter()
        .next()
        .ok_or(ChatError::Unavailable)?;
    Ok(DirectPlan::New {
        peer: placed.0,
        peer_state: placed.1,
    })
}

pub async fn reopen(pg: &PgPool, conversation: Uuid, user: &str) -> ChatResult<()> {
    let mut conn = pg.acquire().await?;
    members::activate(
        &mut conn,
        conversation,
        user,
        &[MemberState::Invited, MemberState::Left],
    )
    .await?;
    Ok(())
}

pub async fn direct(pg: &PgPool, user: &str, plan: DirectPlan) -> ChatResult<Created> {
    let (peer, peer_state) = match plan {
        DirectPlan::Existing(id) => return existing_direct(pg, id, user).await,
        DirectPlan::New { peer, peer_state } => (peer, peer_state),
    };
    let key = dm_key(user, &peer);
    let id = Uuid::now_v7();
    let mut tx = pg.begin().await?;
    if !insert(
        &mut tx,
        id,
        ConversationKind::Direct,
        Some(&key),
        user,
        None,
    )
    .await?
    {
        tx.rollback().await?;
        let id = find_dm(pg, &key)
            .await?
            .ok_or(ChatError::Busy { retry_ms: 200 })?;
        return existing_direct(pg, id, user).await;
    }
    let joining = [
        Joining::creator(user, MemberRole::Member),
        Joining::placed(peer.clone(), peer_state),
    ];
    members::join_many(&mut tx, id, 0, 0, user, &joining).await?;
    members::recount(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Created {
        id,
        rows: Vec::new(),
        recipients: vec![user.to_owned(), peer],
    })
}

async fn existing_direct(pg: &PgPool, id: Uuid, user: &str) -> ChatResult<Created> {
    reopen(pg, id, user).await?;
    Ok(Created {
        id,
        rows: Vec::new(),
        recipients: vec![user.to_owned()],
    })
}

pub async fn saved(pg: &PgPool, user: &str) -> ChatResult<Created> {
    let key = saved_key(user);
    if let Some(id) = find_dm(pg, &key).await? {
        return Ok(Created {
            id,
            rows: Vec::new(),
            recipients: vec![user.to_owned()],
        });
    }
    let id = Uuid::now_v7();
    let mut tx = pg.begin().await?;
    if !insert(&mut tx, id, ConversationKind::Saved, Some(&key), user, None).await? {
        tx.rollback().await?;
        let id = find_dm(pg, &key)
            .await?
            .ok_or(ChatError::Busy { retry_ms: 200 })?;
        return Ok(Created {
            id,
            rows: Vec::new(),
            recipients: vec![user.to_owned()],
        });
    }
    members::join_many(
        &mut tx,
        id,
        0,
        0,
        user,
        &[Joining::creator(user, MemberRole::Owner)],
    )
    .await?;
    members::recount(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Created {
        id,
        rows: Vec::new(),
        recipients: vec![user.to_owned()],
    })
}

pub async fn group(
    pg: &PgPool,
    user: &str,
    placed: Vec<(String, MemberState)>,
    meta: Option<(Vec<u8>, i32)>,
) -> ChatResult<Created> {
    let id = Uuid::now_v7();
    let mut joining = vec![Joining::creator(user, MemberRole::Owner)];
    joining.extend(
        placed
            .into_iter()
            .map(|(member, state)| Joining::placed(member, state)),
    );
    let mut tx = pg.begin().await?;
    insert(&mut tx, id, ConversationKind::Group, None, user, meta).await?;
    members::join_many(&mut tx, id, 0, 0, user, &joining).await?;
    let event = SystemEvent::Created {
        actor: user.to_owned(),
    };
    let row = system::insert(&mut tx, id, &event, None).await?;
    members::recount(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Created {
        id,
        rows: vec![row],
        recipients: joining.into_iter().map(|one| one.user_id).collect(),
    })
}

async fn insert(
    conn: &mut PgConnection,
    id: Uuid,
    kind: ConversationKind,
    dm_key: Option<&str>,
    user: &str,
    meta: Option<(Vec<u8>, i32)>,
) -> ChatResult<bool> {
    let (meta_ct, meta_epoch) = meta.unzip();
    Ok(sqlx::query_file_scalar!(
        "queries/chat/conversations/insert.sql",
        id,
        kind.as_str(),
        dm_key,
        user,
        meta_ct,
        meta_epoch,
    )
    .fetch_optional(&mut *conn)
    .await?
    .is_some())
}

#[cfg(test)]
#[path = "../tests/conversation_create_tests.rs"]
mod tests;
