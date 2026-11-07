pub mod rules;
mod store;
pub mod view;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::conversations::view::ConversationView;
use crate::modules::chat::members::access::{self, Access};
use crate::modules::chat::messages::fanout;
use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::public_chat::{self, delivery};
use crate::modules::chat::{
    ChatCaller, ChatError, ChatResult, ChatState, conversations, limits, media,
};
use crate::modules::rooms::model::Backdrop;

pub use store::{clear_image, image_in};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub backdrop: Backdrop,
    pub by: String,
}

impl Chosen {
    pub fn of(settings: &Value) -> Option<Self> {
        let raw = settings.get("backdrop")?;
        let by = raw.get("by")?.as_str()?.to_owned();
        let backdrop = serde_json::from_value(raw.clone()).ok()?;
        Some(Self { backdrop, by })
    }

    pub fn image(&self) -> Option<Uuid> {
        self.backdrop.image()
    }

    pub fn json(&self) -> Value {
        let mut value = serde_json::to_value(&self.backdrop).unwrap_or(Value::Null);
        if let Value::Object(fields) = &mut value {
            fields.insert("by".to_owned(), Value::String(self.by.clone()));
        }
        value
    }
}

impl Serialize for Chosen {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.json().serialize(serializer)
    }
}

pub async fn set(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatJson(wanted): ChatJson<Option<Backdrop>>,
) -> ChatResult<Json<ConversationView>> {
    public_chat::guard(&chat, conversation)?;
    let access = access::load(&chat.pg, conversation, &caller.user_id)
        .await?
        .active()?;
    rules::may_set(&access)?;
    if wanted.as_ref().is_some_and(|backdrop| !backdrop.known()) {
        return Err(ChatError::Invalid);
    }
    let chosen = wanted.map(|backdrop| Chosen {
        backdrop,
        by: caller.user_id.clone(),
    });
    if chosen != access.backdrop {
        change(&chat, &access, chosen.as_ref()).await?;
    }
    let view = if access.is_public() {
        public_chat::view::of(&chat, &caller.user_id).await?
    } else {
        conversations::view::one(&chat, conversation, &caller.user_id).await?
    };
    Ok(Json(view))
}

async fn change(chat: &ChatState, access: &Access, chosen: Option<&Chosen>) -> ChatResult<()> {
    let user = access.user_id.as_str();
    if let Some(chosen) = chosen {
        if rules::needs_star(access.kind, &chosen.backdrop) {
            starred(chat, user).await?;
        }
        if let Some(media) = chosen.image() {
            media::backdrop::check(chat, Some(access.conversation), user, media).await?;
        }
    }
    let budget = access.conversation.to_string();
    limits::admit_backdrop(chat, &budget).await?;
    let applied = store::apply(chat, access, chosen).await;
    if applied.is_err() {
        limits::refund_backdrop(chat, &budget).await;
    }
    applied?;
    notify(
        chat,
        access.kind,
        access.conversation,
        access.last_seq,
        &access.recipients,
    )
    .await
}

async fn starred(chat: &ChatState, user: &str) -> ChatResult<()> {
    if chat.stars.is_star(user).await? || chat.stars.is_star_now(user).await? {
        Ok(())
    } else {
        Err(ChatError::BackdropStarRequired)
    }
}

pub async fn notify(
    chat: &ChatState,
    kind: ConversationKind,
    conversation: Uuid,
    last_seq: i64,
    recipients: &[String],
) -> ChatResult<()> {
    if kind == ConversationKind::Public {
        delivery::announce(chat, last_seq).await
    } else {
        fanout::bump(chat, conversation, last_seq, recipients).await
    }
}

#[cfg(test)]
#[path = "tests/chat_backdrop_support.rs"]
pub mod support;

#[cfg(test)]
#[path = "tests/chat_backdrop_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/chat_backdrop_image_tests.rs"]
mod image_tests;

#[cfg(test)]
#[path = "tests/chat_backdrop_public_tests.rs"]
mod public_tests;
