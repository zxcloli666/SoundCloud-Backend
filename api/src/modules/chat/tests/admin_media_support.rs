use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::Utc;
use sc_chat_wire::msg::{Class, MessageAad};
use sc_chat_wire::proto::envelope::Body;
use sc_chat_wire::proto::{Attachment, AttachmentKind, Envelope, Meta, Text, Thumb};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::admin::cipher::{BLOCK, PAD_MARK, Role, aad};
use crate::modules::chat::moderation::evidence::{self, StoredMessage};
use crate::modules::chat::moderation::keys::ChatKeys;

const TAG: usize = 16;

pub fn sealed_len(plain_len: usize) -> usize {
    (plain_len + 1 + TAG).div_ceil(BLOCK) * BLOCK
}

pub fn seal_blob(role: Role, key: &[u8; 32], nonce: &[u8; 24], plain: &[u8]) -> Vec<u8> {
    let mut padded = vec![0u8; sealed_len(plain.len()) - TAG];
    padded[..plain.len()].copy_from_slice(plain);
    padded[plain.len()] = PAD_MARK;
    XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &padded,
                aad: &aad(role),
            },
        )
        .expect("a sealed blob")
}

pub struct Upload {
    pub id: Uuid,
    pub plain: Vec<u8>,
    pub key: [u8; 32],
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

impl Upload {
    pub fn new(role: Role, plain: &[u8]) -> Self {
        let key: [u8; 32] = sc_chat_wire::random_bytes().expect("a key");
        let nonce: [u8; 24] = sc_chat_wire::random_bytes().expect("a nonce");
        Self {
            id: Uuid::new_v4(),
            plain: plain.to_vec(),
            ciphertext: seal_blob(role, &key, &nonce, plain),
            key,
            nonce,
        }
    }

    pub fn bytes(&self) -> i64 {
        self.ciphertext.len() as i64
    }

    pub fn object_key(&self) -> String {
        format!("m/{}", self.id.simple())
    }
}

pub fn webp(tag: &str, len: usize) -> Vec<u8> {
    let mut plain = b"RIFF\0\0\0\0WEBPVP8L".to_vec();
    plain.extend(tag.bytes().cycle().take(len));
    plain
}

pub fn image(image: &Upload, thumb: &Upload) -> Attachment {
    Attachment {
        thumb: Some(Thumb {
            media_id: thumb.id.to_string(),
            key: thumb.key.to_vec(),
            nonce: thumb.nonce.to_vec(),
            size: thumb.ciphertext.len() as u64,
            w: 320,
            h: 180,
        }),
        ..single(image, 1600, 900)
    }
}

pub fn single(upload: &Upload, w: u32, h: u32) -> Attachment {
    Attachment {
        media_id: upload.id.to_string(),
        kind: AttachmentKind::Image as i32,
        mime: "image/webp".to_owned(),
        size: upload.ciphertext.len() as u64,
        w,
        h,
        thumbhash: vec![1, 2, 3],
        key: upload.key.to_vec(),
        nonce: upload.nonce.to_vec(),
        sha256_plain: Sha256::digest(&upload.plain).to_vec(),
        sha256_cipher: Sha256::digest(&upload.ciphertext).to_vec(),
        thumb: None,
    }
}

pub fn post_envelope(text: &str, attachments: Vec<Attachment>) -> Envelope {
    Envelope {
        v: sc_chat_wire::msg::ENVELOPE_VERSION,
        body: Some(Body::Text(Text {
            text: text.to_owned(),
            spans: Vec::new(),
        })),
        attachments,
        ..Default::default()
    }
}

pub fn meta_envelope(attachment: Option<Attachment>) -> Envelope {
    let avatar_media_id = attachment
        .as_ref()
        .map(|attachment| attachment.media_id.clone())
        .unwrap_or_default();
    Envelope {
        v: sc_chat_wire::msg::ENVELOPE_VERSION,
        body: Some(Body::Meta(Meta {
            title: "Room meta".to_owned(),
            description: String::new(),
            avatar_media_id,
            backdrop_media_id: String::new(),
        })),
        attachments: attachment.into_iter().collect(),
        ..Default::default()
    }
}

pub fn backdrop_meta_envelope(avatar: Option<Attachment>, backdrop: Attachment) -> Envelope {
    Envelope {
        v: sc_chat_wire::msg::ENVELOPE_VERSION,
        body: Some(Body::Meta(Meta {
            title: "Group meta".to_owned(),
            description: String::new(),
            avatar_media_id: avatar
                .as_ref()
                .map(|attachment| attachment.media_id.clone())
                .unwrap_or_default(),
            backdrop_media_id: backdrop.media_id.clone(),
        })),
        attachments: avatar.into_iter().chain([backdrop]).collect(),
        ..Default::default()
    }
}

pub async fn send(
    pg: &PgPool,
    conversation: Uuid,
    ck: &[u8; 32],
    sender: &str,
    class: Class,
    envelope: &Envelope,
) -> StoredMessage {
    let seq: i64 = sqlx::query_scalar(
        "UPDATE chat.conversations SET last_seq = last_seq + 1, last_at = now() WHERE id = $1 RETURNING last_seq",
    )
    .bind(conversation)
    .fetch_one(pg)
    .await
    .expect("a seq");
    let client_id = Uuid::new_v4();
    let device = Uuid::new_v4();
    let (aad_device, aad_client) = match class {
        Class::Meta => (Uuid::nil(), Uuid::nil()),
        _ => (device, client_id),
    };
    let aad = MessageAad {
        conv: conversation.into_bytes(),
        epoch: 0,
        sender_device: aad_device.into_bytes(),
        client_id: aad_client.into_bytes(),
        class,
    };
    let body = sc_chat_wire::msg::seal(
        ck,
        &aad,
        sc_chat_wire::random_bytes().expect("a nonce"),
        envelope,
    )
    .expect("a sealed body");
    sqlx::query(
        "INSERT INTO chat.messages (conversation_id, seq, client_id, sender_user, sender_device, class, epoch, body)
         VALUES ($1, $2, $3, $4, $5, $6, 0, $7)",
    )
    .bind(conversation)
    .bind(seq)
    .bind(client_id)
    .bind(sender)
    .bind((class != Class::Meta).then_some(device))
    .bind(class.as_str())
    .bind(&body)
    .execute(pg)
    .await
    .expect("a message");
    StoredMessage {
        seq,
        client_id,
        sender_user: sender.to_owned(),
        sender_device: (class != Class::Meta).then_some(device),
        class: class.as_str().to_owned(),
        epoch: 0,
        target_seq: None,
        body,
        created_at: Utc::now(),
    }
}

pub struct Row<'a> {
    pub upload: &'a Upload,
    pub owner: &'a str,
    pub origin: Uuid,
    pub kind: &'a str,
    pub parent: Option<Uuid>,
    pub status: &'a str,
}

pub async fn media_row(pg: &PgPool, row: Row<'_>) {
    sqlx::query(
        "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, parent_id, bytes, status, delete_after)
         VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, CASE WHEN $8 = 'uploaded' THEN now() + interval '1 hour' END)",
    )
    .bind(row.upload.id)
    .bind(row.upload.object_key())
    .bind(row.owner)
    .bind(row.origin)
    .bind(row.kind)
    .bind(row.parent)
    .bind(row.upload.bytes())
    .bind(row.status)
    .execute(pg)
    .await
    .expect("a media row");
}

pub async fn status_of(pg: &PgPool, id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM chat.media WHERE id = $1")
        .bind(id)
        .fetch_optional(pg)
        .await
        .expect("a status read")
}

pub async fn refs_of(pg: &PgPool, conversation: Uuid) -> Vec<(Uuid, i64)> {
    sqlx::query_as(
        "SELECT media_id, seq FROM chat.media_refs WHERE conversation_id = $1 ORDER BY media_id, seq",
    )
    .bind(conversation)
    .fetch_all(pg)
    .await
    .expect("refs")
}

pub async fn hold(pg: &PgPool, media: Uuid, report: Uuid) {
    sqlx::query(
        "INSERT INTO chat.media_holds (media_id, report_id, until) VALUES ($1, $2, now() + interval '30 days')",
    )
    .bind(media)
    .bind(report)
    .execute(pg)
    .await
    .expect("a hold");
}

pub async fn conversation_key(pg: &PgPool, keys: &dyn ChatKeys, conversation: Uuid) -> [u8; 32] {
    let ck: [u8; 32] = sc_chat_wire::random_bytes().expect("a ck");
    let wrapped = keys.wrap_ck(conversation, 0, &ck).expect("a wrapped ck");
    sqlx::query(
        "INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version)
         VALUES ($1, 0, $2, $3)",
    )
    .bind(conversation)
    .bind(wrapped)
    .bind(keys.kek_current() as i16)
    .execute(pg)
    .await
    .expect("a conversation key");
    ck
}

pub async fn backdrop_report(
    chat: &ChatState,
    room: Uuid,
    host: &str,
    reporter: &str,
    meta: &StoredMessage,
    media: Uuid,
) -> Uuid {
    let capture = evidence::capture(
        chat,
        &chat.keks,
        room,
        (meta.seq, meta.seq),
        std::slice::from_ref(meta),
    )
    .await
    .expect("a capture");
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.reports (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight, evidence, evidence_kek_version, kind, media_id)
         VALUES ($1, $2, $3, $4, $5, $5, 'other', 1, $6, $7, 'backdrop', $8)",
    )
    .bind(id)
    .bind(reporter)
    .bind(host)
    .bind(room)
    .bind(meta.seq)
    .bind(capture.sealed)
    .bind(capture.version as i16)
    .bind(media)
    .execute(&chat.pg)
    .await
    .expect("a backdrop report");
    id
}
