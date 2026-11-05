use uuid::Uuid;

use crate::modules::chat::admin::evidence::MediaKind;
use crate::modules::chat::reports::{self, ReportKind, ReportRow};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub struct Filed {
    pub row: ReportRow,
    pub kind: ReportKind,
    pub media_id: Option<Uuid>,
    pub in_room: bool,
}

impl Filed {
    pub fn meta_kind(&self) -> MediaKind {
        if self.kind == ReportKind::Backdrop || self.in_room {
            MediaKind::Backdrop
        } else {
            MediaKind::Avatar
        }
    }

    pub fn backdrop(&self) -> Option<Uuid> {
        self.media_id.filter(|_| self.kind == ReportKind::Backdrop)
    }
}

pub async fn load(chat: &ChatState, id: Uuid) -> ChatResult<Filed> {
    let row = reports::load(chat, id).await?.ok_or(ChatError::NotFound)?;
    let extra = sqlx::query_file!("queries/chat/admin/report_kind.sql", id)
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotFound)?;
    Ok(Filed {
        row,
        kind: ReportKind::parse(&extra.kind)?,
        media_id: extra.media_id,
        in_room: extra.conversation_kind.as_deref() == Some("room"),
    })
}
