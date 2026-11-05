use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::admin::evidence::{self, SealedMedia};
use crate::modules::chat::admin::{audit, cipher, filed};
use crate::modules::chat::body::ChatPath;
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const MIME_WEBP: &str = "image/webp";

pub async fn view(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath((id, media)): ChatPath<(Uuid, Uuid)>,
) -> ChatResult<Response> {
    let image = view_with(&chat, &chat.keks, &admin.admin, id, media).await?;
    Ok((
        [
            (header::CONTENT_TYPE, MIME_WEBP),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        image,
    )
        .into_response())
}

pub async fn head(_: ChatAdmin) -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

pub async fn view_with(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    admin: &str,
    id: Uuid,
    media: Uuid,
) -> ChatResult<Vec<u8>> {
    let report = filed::load(chat, id).await?;
    let bundle = evidence::bundle(keys, &report.row)?;
    let candidates = evidence::sealed_as(&bundle, report.meta_kind(), media);
    if candidates.is_empty() {
        return Err(ChatError::NotFound);
    }
    let stored = sqlx::query_file!("queries/chat/admin/media_row.sql", media)
        .fetch_optional(&chat.pg)
        .await?
        .filter(|row| viewable(&row.status, row.restorable, row.held))
        .ok_or(ChatError::MediaGone)?;
    let store = u16::try_from(stored.store).map_err(|_| ChatError::Internal)?;
    let ciphertext = chat
        .s3
        .get(store, &stored.object_key)
        .await?
        .ok_or(ChatError::MediaGone)?;
    let plain = candidates
        .iter()
        .find_map(|sealed| opened(sealed, &ciphertext))
        .ok_or_else(|| {
            tracing::warn!(report = %id, %media, "a reported chat image does not match its envelope");
            ChatError::MediaGone
        })?;
    audit::record(
        chat,
        admin,
        "view_media",
        &audit::report(id),
        json!({"media": media}),
    )
    .await?;
    Ok(plain)
}

pub fn viewable(status: &str, restorable: bool, held: bool) -> bool {
    match status {
        "uploaded" | "attached" | "held" => true,
        "deleting" => held || restorable,
        _ => false,
    }
}

pub fn opened(sealed: &SealedMedia, ciphertext: &[u8]) -> Option<Vec<u8>> {
    if !matches_digest(&sealed.sha256_cipher, ciphertext) {
        return None;
    }
    let plain = cipher::open(
        sealed.shown.kind.role(),
        &sealed.key,
        &sealed.nonce,
        ciphertext,
    )?;
    matches_digest(&sealed.sha256_plain, &plain).then_some(plain)
}

fn matches_digest(expected: &[u8], bytes: &[u8]) -> bool {
    expected.is_empty() || Sha256::digest(bytes)[..] == *expected
}
