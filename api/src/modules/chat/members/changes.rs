use sqlx::{PgConnection, PgPool};

use crate::modules::chat::conversations::remove;
use crate::modules::chat::epochs;
use crate::modules::chat::members::access::Access;
use crate::modules::chat::members::roles;
use crate::modules::chat::members::store::{self as members, Joining};
use crate::modules::chat::messages::fanout;
use crate::modules::chat::messages::system::{self, SystemEvent};
use crate::modules::chat::model::{GROUP_MEMBERS_MAX, MemberRole, MemberState, MessageRow};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

const LIVE: [MemberState; 2] = [MemberState::Active, MemberState::Invited];

#[derive(Default)]
pub struct Change {
    pub rows: Vec<MessageRow>,
    pub recipients: Vec<String>,
    pub added: Vec<String>,
    pub deleted: bool,
}

impl Change {
    fn to(recipients: Vec<String>) -> Self {
        Self {
            recipients,
            ..Self::default()
        }
    }
}

async fn lock_chat(conn: &mut PgConnection, access: &Access) -> ChatResult<members::Locked> {
    members::lock(conn, access.conversation)
        .await?
        .ok_or(ChatError::NotFound)
}

pub async fn leave(pg: &PgPool, access: &Access) -> ChatResult<Change> {
    if !access.is_group() {
        return Err(ChatError::Invalid);
    }
    let me = access.user_id.as_str();
    let mut change = Change::to(access.recipients_with(me));
    let mut tx = pg.begin().await?;
    lock_chat(&mut tx, access).await?;
    let Some(role) =
        members::set_state(&mut tx, access.conversation, me, MemberState::Left, &LIVE).await?
    else {
        tx.rollback().await?;
        return Ok(change);
    };
    if members::recount(&mut tx, access.conversation).await? == 0 {
        change.deleted = remove::shred(&mut tx, access.conversation).await?;
        tx.commit().await?;
        return Ok(change);
    }
    if role == MemberRole::Owner {
        change.rows.extend(hand_over(&mut tx, access).await?);
    }
    epochs::mark_rekey_in(&mut tx, access.conversation).await?;
    let event = SystemEvent::Left {
        actor: me.to_owned(),
    };
    change
        .rows
        .push(system::insert(&mut tx, access.conversation, &event, None).await?);
    tx.commit().await?;
    Ok(change)
}

async fn hand_over(conn: &mut PgConnection, access: &Access) -> ChatResult<Option<MessageRow>> {
    let Some(heir) = members::successor(conn, access.conversation, &access.user_id).await? else {
        return Ok(None);
    };
    members::set_role(conn, access.conversation, &heir, MemberRole::Owner).await?;
    let event = SystemEvent::Role {
        actor: access.user_id.clone(),
        target: heir,
        role: MemberRole::Owner,
    };
    Ok(Some(
        system::insert(conn, access.conversation, &event, None).await?,
    ))
}

pub async fn remove(pg: &PgPool, access: &Access, target: &str, ban: bool) -> ChatResult<Change> {
    let actor = access.clone().active()?.admin()?;
    if target == actor.user_id {
        return Err(ChatError::Invalid);
    }
    let mut change = Change::to(actor.recipients_with(target));
    let mut tx = pg.begin().await?;
    lock_chat(&mut tx, &actor).await?;
    let (role, state) = members::standing(&mut tx, actor.conversation, target)
        .await?
        .ok_or(ChatError::NotFound)?;
    if !roles::may_remove(actor.role, role) {
        return Err(ChatError::NotMember);
    }
    let (next, from): (MemberState, &[MemberState]) = if ban {
        (
            MemberState::Banned,
            &[
                MemberState::Active,
                MemberState::Invited,
                MemberState::Left,
                MemberState::Kicked,
            ],
        )
    } else {
        (MemberState::Kicked, &LIVE)
    };
    if members::set_state(&mut tx, actor.conversation, target, next, from)
        .await?
        .is_none()
    {
        tx.rollback().await?;
        return if state == next {
            Ok(change)
        } else {
            Err(ChatError::NotFound)
        };
    }
    if LIVE.contains(&state) {
        epochs::mark_rekey_in(&mut tx, actor.conversation).await?;
    }
    let event = if ban {
        SystemEvent::Banned {
            actor: actor.user_id.clone(),
            target: target.to_owned(),
        }
    } else {
        SystemEvent::Kicked {
            actor: actor.user_id.clone(),
            target: target.to_owned(),
        }
    };
    change
        .rows
        .push(system::insert(&mut tx, actor.conversation, &event, None).await?);
    members::recount(&mut tx, actor.conversation).await?;
    tx.commit().await?;
    Ok(change)
}

pub async fn unban(pg: &PgPool, access: &Access, target: &str) -> ChatResult<Change> {
    let actor = access.clone().active()?.admin()?;
    let mut conn = pg.acquire().await?;
    members::set_state(
        &mut conn,
        actor.conversation,
        target,
        MemberState::Left,
        &[MemberState::Banned],
    )
    .await?;
    Ok(Change::to(vec![actor.user_id]))
}

pub async fn set_role(
    pg: &PgPool,
    access: &Access,
    target: &str,
    role: MemberRole,
) -> ChatResult<Change> {
    let actor = access.clone().active()?.admin()?;
    if actor.role != MemberRole::Owner || target == actor.user_id {
        return Err(ChatError::NotMember);
    }
    let mut change = Change::to(actor.recipients.clone());
    let mut tx = pg.begin().await?;
    lock_chat(&mut tx, &actor).await?;
    if role == MemberRole::Owner
        && !members::step_down(&mut tx, actor.conversation, &actor.user_id).await?
    {
        tx.rollback().await?;
        return Err(ChatError::NotMember);
    }
    if !members::set_role(&mut tx, actor.conversation, target, role).await? {
        tx.rollback().await?;
        return Err(ChatError::NotFound);
    }
    let event = SystemEvent::Role {
        actor: actor.user_id.clone(),
        target: target.to_owned(),
        role,
    };
    change
        .rows
        .push(system::insert(&mut tx, actor.conversation, &event, None).await?);
    tx.commit().await?;
    Ok(change)
}

pub async fn accept(pg: &PgPool, access: &Access) -> ChatResult<Change> {
    let me = access.user_id.as_str();
    let mut change = Change::to(access.recipients.clone());
    let mut tx = pg.begin().await?;
    lock_chat(&mut tx, access).await?;
    if !members::activate(&mut tx, access.conversation, me, &[MemberState::Invited]).await? {
        tx.rollback().await?;
        return match access.state {
            MemberState::Active => Ok(change),
            _ => Err(ChatError::NotMember),
        };
    }
    if access.is_group() {
        let event = SystemEvent::Joined {
            actor: me.to_owned(),
        };
        change
            .rows
            .push(system::insert(&mut tx, access.conversation, &event, None).await?);
    }
    members::recount(&mut tx, access.conversation).await?;
    tx.commit().await?;
    Ok(change)
}

pub async fn decline(pg: &PgPool, access: &Access) -> ChatResult<Change> {
    let me = access.user_id.as_str();
    let mut conn = pg.acquire().await?;
    let declined = members::set_state(
        &mut conn,
        access.conversation,
        me,
        MemberState::Left,
        &[MemberState::Invited],
    )
    .await?;
    if declined.is_none() && access.state != MemberState::Left {
        return Err(ChatError::NotMember);
    }
    Ok(Change::to(vec![me.to_owned()]))
}

pub async fn add(
    pg: &PgPool,
    access: &Access,
    placed: Vec<(String, MemberState)>,
) -> ChatResult<Change> {
    let actor = access.clone().active()?.admin()?;
    let joining: Vec<Joining> = placed
        .into_iter()
        .map(|(user, state)| Joining::placed(user, state))
        .collect();
    let mut change = Change::to(actor.recipients.clone());
    if joining.is_empty() {
        return Ok(change);
    }
    let mut tx = pg.begin().await?;
    let chat_row = lock_chat(&mut tx, &actor).await?;
    let hidden = !chat_row.settings.history_visible;
    let joined_epoch = if hidden {
        chat_row.key_epoch.saturating_add(1)
    } else {
        0
    };
    let live = members::live_total(&mut tx, actor.conversation).await?;
    if usize::try_from(live).unwrap_or(usize::MAX) + joining.len() > GROUP_MEMBERS_MAX {
        tx.rollback().await?;
        return Err(ChatError::Invalid);
    }
    let joined = members::join_many(
        &mut tx,
        actor.conversation,
        chat_row.last_seq,
        joined_epoch,
        &actor.user_id,
        &joining,
    )
    .await?;
    if joined.is_empty() {
        tx.rollback().await?;
        return Ok(change);
    }
    if hidden {
        epochs::mark_rekey_in(&mut tx, actor.conversation).await?;
    }
    let targets: Vec<String> = joined.into_iter().map(|one| one.user_id).collect();
    let event = SystemEvent::Added {
        actor: actor.user_id.clone(),
        targets: targets.clone(),
    };
    change
        .rows
        .push(system::insert(&mut tx, actor.conversation, &event, None).await?);
    members::recount(&mut tx, actor.conversation).await?;
    tx.commit().await?;
    change.recipients.extend(targets.iter().cloned());
    change.added = targets;
    Ok(change)
}

pub async fn publish(chat: &ChatState, access: &Access, change: Change) -> ChatResult<()> {
    if change.rows.is_empty() {
        return fanout::bump(
            chat,
            access.conversation,
            access.last_seq,
            &change.recipients,
        )
        .await;
    }
    fanout::publish(chat, &change.rows, &change.recipients).await
}

#[cfg(test)]
#[path = "../tests/member_changes_tests.rs"]
mod tests;
