use chrono::{DateTime, Duration, Utc};
use sc_chat_wire::msg::Class;
use uuid::Uuid;

use crate::modules::chat::conversations::create::META_MAX;
use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::{EDIT_WINDOW_HOURS, SendMessage};
use crate::modules::chat::model::{ConversationKind, MemberState};
use crate::modules::chat::{ChatError, ChatResult};

pub const BODY_MAX: usize = 48 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub client_id: Uuid,
    pub class: Class,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub sender_user: String,
    pub class: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub deleted: bool,
}

pub fn outgoing(input: SendMessage) -> ChatResult<Outgoing> {
    let class = Class::parse(&input.class).ok_or(ChatError::Invalid)?;
    let cap = if class == Class::Meta {
        META_MAX
    } else {
        BODY_MAX
    };
    if class == Class::System || input.body.is_empty() || input.body.len() > cap {
        return Err(ChatError::Invalid);
    }
    if input.epoch < 0 || input.target_seq.is_some_and(|seq| seq < 1) {
        return Err(ChatError::Invalid);
    }
    if needs_target(class) != input.target_seq.is_some() {
        return Err(ChatError::Invalid);
    }
    Ok(Outgoing {
        client_id: input.client_id,
        class,
        epoch: input.epoch,
        target_seq: input.target_seq,
        body: input.body,
    })
}

pub fn needs_target(class: Class) -> bool {
    matches!(class, Class::Edit | Class::React | Class::Pin)
}

pub fn may_post(access: &Access, class: Class) -> ChatResult<()> {
    if access.state != MemberState::Active {
        return Err(ChatError::NotMember);
    }
    match class {
        Class::Meta if access.kind != ConversationKind::Group => Err(ChatError::Invalid),
        Class::Meta | Class::Pin
            if (access.is_group() || access.is_room()) && !access.is_admin() =>
        {
            Err(ChatError::NotMember)
        }
        Class::Post if access.settings.admins_only && access.is_group() && !access.is_admin() => {
            Err(ChatError::NotMember)
        }
        Class::System => Err(ChatError::Invalid),
        _ => Ok(()),
    }
}

pub fn inherited_expiry(
    class: Class,
    user: &str,
    target: Option<&Target>,
    now: DateTime<Utc>,
) -> ChatResult<Option<DateTime<Utc>>> {
    if !needs_target(class) {
        return Ok(None);
    }
    let target = target.ok_or(ChatError::NotFound)?;
    if target.class != Class::Post.as_str() {
        return Err(ChatError::Invalid);
    }
    if target.deleted || target.expires_at.is_some_and(|at| at <= now) {
        return Err(ChatError::NotFound);
    }
    if class == Class::Edit {
        if target.sender_user != user {
            return Err(ChatError::NotMember);
        }
        if target.created_at + Duration::hours(EDIT_WINDOW_HOURS) < now {
            return Err(ChatError::Invalid);
        }
    }
    Ok(target.expires_at)
}

pub fn expiry(
    class: Class,
    ttl_s: Option<i32>,
    inherited: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match class {
        Class::Post => ttl_s.map(|seconds| at + Duration::seconds(i64::from(seconds))),
        Class::Edit | Class::React | Class::Pin => inherited,
        Class::Meta | Class::System => None,
    }
}

#[cfg(test)]
#[path = "../tests/message_rules_tests.rs"]
mod tests;
