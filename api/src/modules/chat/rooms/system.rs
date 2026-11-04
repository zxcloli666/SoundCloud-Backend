use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::model::MessageRow;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedTrack {
    pub urn: String,
    pub title: String,
    pub artist: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemRow {
    Join {
        user_id: String,
        name: String,
    },
    Leave {
        user_id: String,
        name: String,
    },
    Kick {
        user_id: String,
        name: String,
        by: String,
        ban: bool,
    },
    Host {
        user_id: String,
        name: String,
    },
    Moderator {
        user_id: String,
        name: String,
        by: String,
        on: bool,
    },
    Mute {
        user_id: String,
        name: String,
        by: String,
        until_ms: Option<i64>,
    },
    Title {
        by: String,
        title: Option<String>,
    },
    Track {
        urn: String,
        title: Option<String>,
        artist: Option<String>,
        artwork_url: Option<String>,
    },
    Suggest {
        user_id: String,
        name: String,
        track: SuggestedTrack,
    },
}

pub struct Authored {
    pub sender: String,
    pub row: SystemRow,
}

pub async fn post(
    chat: &ChatState,
    conversation: Uuid,
    rows: &[Authored],
    rekey: bool,
) -> ChatResult<Option<i64>> {
    let count = i64::try_from(rows.len()).map_err(|_| ChatError::Invalid)?;
    let mut tx = chat.pg.begin().await?;
    let Some(advanced) =
        sqlx::query_file!("queries/chat/rooms/advance.sql", conversation, count, rekey)
            .fetch_optional(&mut *tx)
            .await?
    else {
        return Ok(None);
    };
    let mut stored = Vec::with_capacity(rows.len());
    for (seq, authored) in (advanced.last_seq - count + 1..).zip(rows) {
        let body = serde_json::to_vec(&authored.row).map_err(|_| ChatError::Internal)?;
        let client_id = Uuid::now_v7();
        let created_at = sqlx::query_file_scalar!(
            "queries/chat/rooms/insert_system.sql",
            conversation,
            seq,
            client_id,
            authored.sender,
            advanced.key_epoch,
            &body
        )
        .fetch_one(&mut *tx)
        .await?;
        stored.push(MessageRow {
            conversation_id: conversation,
            seq,
            client_id,
            sender_user: authored.sender.clone(),
            sender_device: None,
            class: "system".to_owned(),
            epoch: advanced.key_epoch,
            target_seq: None,
            body: Some(body),
            created_at,
            expires_at: None,
        });
    }
    tx.commit().await?;
    for row in stored {
        chat.hot.put(row);
    }
    Ok(Some(advanced.last_seq))
}
