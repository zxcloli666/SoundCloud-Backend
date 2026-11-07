use std::collections::HashMap;

use sc_chat_wire::msg::Class;
use sc_chat_wire::proto::envelope::Body;
use sc_chat_wire::proto::{Attachment, AttachmentKind, Envelope, Thumb};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::admin::cipher::{KEY_LEN, NONCE_LEN, Role};
use crate::modules::chat::media::limits::{ALBUM_MAX, IMAGE_MAX_BYTES, THUMB_MAX_BYTES};
use crate::modules::chat::moderation::evidence::{self, Bundle, Line};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::reports::ReportRow;
use crate::modules::chat::{ChatError, ChatResult};

pub const SHA256_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Thumb,
    Avatar,
    Backdrop,
}

impl MediaKind {
    fn stored(kind: &str) -> Option<Self> {
        match kind {
            "avatar" => Some(MediaKind::Avatar),
            "backdrop" => Some(MediaKind::Backdrop),
            _ => None,
        }
    }

    fn is_meta(self) -> bool {
        matches!(self, MediaKind::Avatar | MediaKind::Backdrop)
    }

    pub fn role(self) -> Role {
        match self {
            MediaKind::Thumb => Role::Thumb,
            MediaKind::Image | MediaKind::Avatar | MediaKind::Backdrop => Role::Image,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LineMedia {
    pub id: Uuid,
    pub kind: MediaKind,
    pub w: u32,
    pub h: u32,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct EvidenceLine {
    #[serde(flatten)]
    pub line: Line,
    pub media: Vec<LineMedia>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedMedia {
    pub shown: LineMedia,
    pub key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub sha256_cipher: Vec<u8>,
    pub sha256_plain: Vec<u8>,
}

pub fn bundle(keys: &dyn ChatKeys, report: &ReportRow) -> ChatResult<Bundle> {
    let (Some(sealed), Some(version)) = (&report.evidence, report.evidence_kek_version) else {
        return Err(ChatError::NotFound);
    };
    let version = u16::try_from(version).map_err(|_| ChatError::Internal)?;
    evidence::open(keys, version, sealed)
}

pub fn shown_lines(lines: Vec<Line>, bundle: &Bundle, meta: MediaKind) -> Vec<EvidenceLine> {
    lines
        .into_iter()
        .zip(per_message(bundle, meta))
        .map(|(line, media)| EvidenceLine {
            line,
            media: media
                .into_iter()
                .map(|sealed| sealed.shown)
                .filter(|shown| shown.kind != MediaKind::Thumb)
                .collect(),
        })
        .collect()
}

pub async fn label_stored(pg: &PgPool, lines: &mut [EvidenceLine]) -> ChatResult<()> {
    let ids: Vec<Uuid> = lines
        .iter()
        .flat_map(|line| &line.media)
        .filter(|media| media.kind.is_meta())
        .map(|media| media.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let stored: HashMap<Uuid, MediaKind> =
        sqlx::query_file!("queries/chat/admin/media_kinds.sql", &ids)
            .fetch_all(pg)
            .await?
            .into_iter()
            .filter_map(|row| MediaKind::stored(&row.kind).map(|kind| (row.id, kind)))
            .collect();
    for media in lines.iter_mut().flat_map(|line| line.media.iter_mut()) {
        if let Some(kind) = stored.get(&media.id).filter(|_| media.kind.is_meta()) {
            media.kind = *kind;
        }
    }
    Ok(())
}

pub fn sealed_as(bundle: &Bundle, meta: MediaKind, id: Uuid) -> Vec<SealedMedia> {
    per_message(bundle, meta)
        .into_iter()
        .flatten()
        .filter(|sealed| sealed.shown.id == id)
        .collect()
}

pub fn reported_media(bundle: &Bundle) -> Vec<Uuid> {
    let reported = bundle.seq_from..=bundle.seq_to;
    bundle
        .messages
        .iter()
        .zip(per_message(bundle, MediaKind::Avatar))
        .filter(|(message, _)| reported.contains(&message.seq))
        .flat_map(|(_, media)| media)
        .filter(|sealed| sealed.shown.kind != MediaKind::Thumb)
        .map(|sealed| sealed.shown.id)
        .collect()
}

pub fn per_message(bundle: &Bundle, meta: MediaKind) -> Vec<Vec<SealedMedia>> {
    let keys = evidence::bundle_keys(bundle);
    bundle
        .messages
        .iter()
        .map(|message| {
            evidence::envelope(bundle.conversation_id, message, &keys)
                .map(|envelope| carried(&message.class, &envelope, meta))
                .unwrap_or_default()
        })
        .collect()
}

pub fn carried(class: &str, envelope: &Envelope, meta: MediaKind) -> Vec<SealedMedia> {
    match Class::parse(class) {
        Some(Class::Post) => envelope
            .attachments
            .iter()
            .filter(|attachment| image_ok(attachment))
            .take(ALBUM_MAX)
            .flat_map(posted)
            .collect(),
        Some(Class::Meta) => meta_images(envelope, meta),
        _ => Vec::new(),
    }
}

fn posted(attachment: &Attachment) -> Vec<SealedMedia> {
    let image = sealed(attachment, MediaKind::Image);
    let thumb = attachment.thumb.as_ref().and_then(thumb);
    image.into_iter().chain(thumb).collect()
}

fn meta_images(envelope: &Envelope, kind: MediaKind) -> Vec<SealedMedia> {
    let Some(Body::Meta(meta)) = &envelope.body else {
        return Vec::new();
    };
    let avatar = meta_image(envelope, &meta.avatar_media_id, kind);
    let backdrop = (meta.backdrop_media_id != meta.avatar_media_id)
        .then(|| meta_image(envelope, &meta.backdrop_media_id, MediaKind::Backdrop))
        .flatten();
    avatar.into_iter().chain(backdrop).collect()
}

fn meta_image(envelope: &Envelope, id: &str, kind: MediaKind) -> Option<SealedMedia> {
    if id.is_empty() {
        return None;
    }
    envelope
        .attachments
        .iter()
        .find(|attachment| attachment.media_id == id)
        .filter(|attachment| sealed_ok(attachment, IMAGE_MAX_BYTES) && attachment.thumb.is_none())
        .and_then(|attachment| sealed(attachment, kind))
}

fn image_ok(attachment: &Attachment) -> bool {
    sealed_ok(attachment, IMAGE_MAX_BYTES) && attachment.thumb.as_ref().is_some_and(thumb_ok)
}

fn sealed_ok(attachment: &Attachment, cap: u64) -> bool {
    attachment.kind == AttachmentKind::Image as i32
        && Uuid::parse_str(&attachment.media_id).is_ok()
        && keyed(&attachment.key, &attachment.nonce)
        && (1..=cap).contains(&attachment.size)
        && attachment.w > 0
        && attachment.h > 0
        && attachment.sha256_cipher.len() == SHA256_LEN
}

fn thumb_ok(thumb: &Thumb) -> bool {
    Uuid::parse_str(&thumb.media_id).is_ok()
        && keyed(&thumb.key, &thumb.nonce)
        && (1..=THUMB_MAX_BYTES).contains(&thumb.size)
        && thumb.w > 0
        && thumb.h > 0
}

fn keyed(key: &[u8], nonce: &[u8]) -> bool {
    key.len() == KEY_LEN && nonce.len() == NONCE_LEN
}

fn sealed(attachment: &Attachment, kind: MediaKind) -> Option<SealedMedia> {
    Some(SealedMedia {
        shown: LineMedia {
            id: Uuid::parse_str(&attachment.media_id).ok()?,
            kind,
            w: attachment.w,
            h: attachment.h,
            size: attachment.size,
        },
        key: attachment.key.clone(),
        nonce: attachment.nonce.clone(),
        sha256_cipher: attachment.sha256_cipher.clone(),
        sha256_plain: attachment.sha256_plain.clone(),
    })
}

fn thumb(thumb: &Thumb) -> Option<SealedMedia> {
    Some(SealedMedia {
        shown: LineMedia {
            id: Uuid::parse_str(&thumb.media_id).ok()?,
            kind: MediaKind::Thumb,
            w: thumb.w,
            h: thumb.h,
            size: thumb.size,
        },
        key: thumb.key.clone(),
        nonce: thumb.nonce.clone(),
        sha256_cipher: Vec::new(),
        sha256_plain: Vec::new(),
    })
}
