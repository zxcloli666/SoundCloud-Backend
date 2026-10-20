use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use chat_wire::msg::{Class, MessageAad};
use chat_wire::proto::envelope::Body;
use chat_wire::proto::{Envelope, Text};
use chat_wire::seal::ResponseKey;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::config::ModerationConfig;
use crate::modules::chat::moderation::Moderator;
use crate::modules::chat::moderation::client::{AiClient, KEY_FILE};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const AI_KEY: &str = "sk-test-moderation-key-7f3a9c";
pub const ADMIN_TOKEN: &str = "admin-token-for-tests-41d2";

pub struct TestKeys {
    pub kek_current: u16,
    pub reports_current: u16,
}

impl Default for TestKeys {
    fn default() -> Self {
        Self {
            kek_current: 1,
            reports_current: 1,
        }
    }
}

fn kek(version: u16) -> [u8; 32] {
    [u8::try_from(version).unwrap_or(0xee); 32]
}

fn reports_kek(version: u16) -> ResponseKey {
    ResponseKey::from_bytes([0x80 | u8::try_from(version).unwrap_or(0x7f); 32])
}

fn evidence_aad(version: u16) -> [u8; 32] {
    [u8::try_from(version).unwrap_or(0xee); 32]
}

impl ChatKeys for TestKeys {
    fn kek_current(&self) -> u16 {
        self.kek_current
    }

    fn reports_current(&self) -> u16 {
        self.reports_current
    }

    fn wrap_ck(&self, conversation: Uuid, epoch: i32, ck: &[u8; 32]) -> ChatResult<Vec<u8>> {
        chat_wire::ck::wrap(
            &kek(self.kek_current),
            self.kek_current,
            conversation.as_bytes(),
            epoch as u32,
            ck,
            chat_wire::random_bytes().map_err(|_| ChatError::Internal)?,
        )
        .map_err(|_| ChatError::Internal)
    }

    fn unwrap_ck(
        &self,
        kek_version: u16,
        conversation: Uuid,
        epoch: i32,
        wrapped: &[u8],
    ) -> ChatResult<[u8; 32]> {
        chat_wire::ck::unwrap(
            &kek(kek_version),
            kek_version,
            conversation.as_bytes(),
            epoch as u32,
            wrapped,
        )
        .map_err(|_| ChatError::Internal)
    }

    fn seal_evidence(&self, evidence: &[u8]) -> ChatResult<Vec<u8>> {
        reports_kek(self.reports_current)
            .seal(
                &evidence_aad(self.reports_current),
                chat_wire::random_bytes().map_err(|_| ChatError::Internal)?,
                evidence,
            )
            .map_err(|_| ChatError::Internal)
    }

    fn open_evidence(&self, version: u16, sealed: &[u8]) -> ChatResult<Vec<u8>> {
        reports_kek(version)
            .open(&evidence_aad(version), sealed)
            .map_err(|_| ChatError::Internal)
    }
}

pub fn keys_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scd-chat-b5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp keys dir");
        std::fs::write(dir.join(KEY_FILE), format!("{AI_KEY}\n")).expect("a key file");
        std::fs::write(
            dir.join(crate::modules::chat::admin::auth::TOKEN_FILE),
            ADMIN_TOKEN,
        )
        .expect("a token file");
        dir
    })
    .clone()
}

pub fn offline_redis() -> deadpool_redis::Pool {
    crate::redis::connect_url("redis://127.0.0.1:1").expect("a lazy redis pool")
}

pub fn state(pg: PgPool, ai: Option<&str>) -> Arc<ChatState> {
    state_with(pg, offline_redis(), ai)
}

pub fn state_with(pg: PgPool, redis: deadpool_redis::Pool, ai: Option<&str>) -> Arc<ChatState> {
    let mut chat = Arc::into_inner(chat_state(pg, redis, true)).expect("a fresh state");
    let dir = keys_dir();
    let moderation = ModerationConfig {
        enabled: ai.is_some(),
        url: ai.map(str::to_owned),
        model: ai.map(|_| "fake-moderator".to_owned()),
    };
    let client = AiClient::load(&moderation, &dir)
        .expect("a moderation client")
        .with_timing(Duration::from_millis(800), Duration::from_millis(20));
    chat.moderator = Moderator::new(client);
    chat.config.moderation = moderation;
    chat.config.keys_dir = dir;
    Arc::new(chat)
}

pub async fn aged_user(pg: &PgPool, user: &str, days: i32) {
    sqlx::query(
        "INSERT INTO chat.users (user_id, first_seen_at) VALUES ($1, now() - $2::int * interval '1 day')
         ON CONFLICT (user_id) DO UPDATE SET first_seen_at = EXCLUDED.first_seen_at",
    )
    .bind(user)
    .bind(days)
    .execute(pg)
    .await
    .expect("a chat user");
}

pub async fn conversation(pg: &PgPool, kind: &str, members: &[(&str, bool)]) -> Uuid {
    let id = Uuid::now_v7();
    let dm_key = (kind == "direct").then(|| format!("dm:{id}"));
    sqlx::query(
        "INSERT INTO chat.conversations (id, kind, dm_key, room_ref, created_by, member_count)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(kind)
    .bind(dm_key)
    .bind((kind == "room").then_some("ROOM42"))
    .bind(members.first().map(|(user, _)| *user).unwrap_or("0"))
    .bind(members.len() as i32)
    .execute(pg)
    .await
    .expect("a conversation");
    for (index, (user, accepted)) in members.iter().enumerate() {
        sqlx::query(
            "INSERT INTO chat.members (conversation_id, user_id, role, state, accepted, invited_by)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(*user)
        .bind(if index == 0 { "owner" } else { "member" })
        .bind(if *accepted { "active" } else { "invited" })
        .bind(*accepted)
        .bind((!accepted).then(|| members[0].0))
        .execute(pg)
        .await
        .expect("a member");
    }
    id
}

pub async fn conversation_key(pg: &PgPool, keys: &TestKeys, conversation: Uuid) -> [u8; 32] {
    let ck: [u8; 32] = chat_wire::random_bytes().expect("a ck");
    let wrapped = keys.wrap_ck(conversation, 0, &ck).expect("a wrapped ck");
    sqlx::query(
        "INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version)
         VALUES ($1, 0, $2, $3)",
    )
    .bind(conversation)
    .bind(wrapped)
    .bind(keys.kek_current as i16)
    .execute(pg)
    .await
    .expect("a conversation key");
    ck
}

pub async fn post(pg: &PgPool, conversation: Uuid, ck: &[u8; 32], sender: &str, text: &str) -> i64 {
    let seq: i64 = sqlx::query_scalar(
        "UPDATE chat.conversations SET last_seq = last_seq + 1, last_at = now() WHERE id = $1 RETURNING last_seq",
    )
    .bind(conversation)
    .fetch_one(pg)
    .await
    .expect("a seq");
    let client_id = Uuid::new_v4();
    let device = Uuid::new_v4();
    let envelope = Envelope {
        v: chat_wire::msg::ENVELOPE_VERSION,
        body: Some(Body::Text(Text {
            text: text.to_owned(),
            spans: Vec::new(),
        })),
        ..Default::default()
    };
    let aad = MessageAad {
        conv: conversation.into_bytes(),
        epoch: 0,
        sender_device: device.into_bytes(),
        client_id: client_id.into_bytes(),
        class: Class::Post,
    };
    let body = chat_wire::msg::seal(
        ck,
        &aad,
        chat_wire::random_bytes().expect("a nonce"),
        &envelope,
    )
    .expect("a sealed body");
    sqlx::query(
        "INSERT INTO chat.messages (conversation_id, seq, client_id, sender_user, sender_device, class, epoch, body)
         VALUES ($1, $2, $3, $4, $5, 'post', 0, $6)",
    )
    .bind(conversation)
    .bind(seq)
    .bind(client_id)
    .bind(sender)
    .bind(device)
    .bind(body)
    .execute(pg)
    .await
    .expect("a message");
    seq
}

pub async fn body_of(pg: &PgPool, conversation: Uuid, seq: i64) -> Option<Vec<u8>> {
    sqlx::query_scalar("SELECT body FROM chat.messages WHERE conversation_id = $1 AND seq = $2")
        .bind(conversation)
        .bind(seq)
        .fetch_one(pg)
        .await
        .expect("a message row")
}

pub async fn audit_actions(pg: &PgPool, target: &str) -> Vec<(String, String, Value)> {
    sqlx::query_as(
        "SELECT admin, action, detail FROM chat.admin_audit WHERE target = $1 ORDER BY created_at, id",
    )
    .bind(target)
    .fetch_all(pg)
    .await
    .expect("audit rows")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeModel {
    Honest,
    Obedient,
    Sloppy,
    FailsFirst(usize),
    AlwaysDown,
    Slow,
}

#[derive(Clone)]
pub struct FakeAi {
    pub url: String,
    pub calls: Arc<AtomicUsize>,
    pub in_flight: Arc<AtomicUsize>,
    pub peak: Arc<AtomicUsize>,
    pub authorization: Arc<Mutex<Vec<String>>>,
    pub requests: Arc<Mutex<Vec<Value>>>,
}

struct Fake {
    model: FakeModel,
    probe: FakeAi,
}

pub async fn fake_ai(model: FakeModel) -> FakeAi {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a fake ai port");
    let address = listener.local_addr().expect("an address");
    let probe = FakeAi {
        url: format!("http://{address}"),
        calls: Arc::default(),
        in_flight: Arc::default(),
        peak: Arc::default(),
        authorization: Arc::default(),
        requests: Arc::default(),
    };
    let app = Router::new()
        .route("/v1/chat/completions", axum::routing::post(complete))
        .with_state(Arc::new(Fake {
            model,
            probe: probe.clone(),
        }));
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the fake ai serves");
    });
    probe
}

async fn complete(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> axum::response::Response {
    let call = fake.probe.calls.fetch_add(1, Ordering::SeqCst) + 1;
    let now = fake.probe.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    fake.probe.peak.fetch_max(now, Ordering::SeqCst);
    if let Some(value) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        fake.probe
            .authorization
            .lock()
            .expect("a lock")
            .push(value.to_owned());
    }
    fake.probe
        .requests
        .lock()
        .expect("a lock")
        .push(request.clone());
    tokio::time::sleep(Duration::from_millis(60)).await;
    let answer = match fake.model {
        FakeModel::AlwaysDown => None,
        FakeModel::FailsFirst(failures) if call <= failures => None,
        FakeModel::Slow => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Some(honest(&request))
        }
        FakeModel::Obedient => Some(obedient(&request).unwrap_or_else(|| honest(&request))),
        FakeModel::Sloppy => Some(sloppy(&request)),
        FakeModel::Honest | FakeModel::FailsFirst(_) => Some(honest(&request)),
    };
    fake.probe.in_flight.fetch_sub(1, Ordering::SeqCst);
    match answer {
        None => (StatusCode::SERVICE_UNAVAILABLE, "down").into_response(),
        Some(content) => Json(json!({
            "id": "fake",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": content}}]
        }))
        .into_response(),
    }
}

fn user_prompt(request: &Value) -> String {
    request["messages"][1]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn target_seqs(prompt: &str) -> Vec<i64> {
    prompt
        .lines()
        .find_map(|line| line.strip_prefix("Seq numbers of the target's REPORTED messages: "))
        .and_then(|rest| serde_json::from_str(rest.trim_end_matches('.')).ok())
        .unwrap_or_default()
}

fn reported_texts(prompt: &str) -> Vec<(i64, String)> {
    prompt
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["kind"] == "REPORTED" && entry["from"] == "target")
        .map(|entry| {
            (
                entry["seq"].as_i64().unwrap_or_default(),
                entry["text"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn verdict(genuine: bool, category: &str, confidence: f64, quotes: Vec<i64>) -> String {
    json!({
        "genuine": genuine,
        "category": category,
        "severity": if genuine { "high" } else { "low" },
        "recommended_action": if genuine { "delete" } else { "none" },
        "confidence": confidence,
        "quote_seq": quotes,
        "rationale": "fake model"
    })
    .to_string()
}

fn honest(request: &Value) -> String {
    let prompt = user_prompt(request);
    let reported = reported_texts(&prompt);
    let pick = |needle: &str| -> Vec<i64> {
        reported
            .iter()
            .filter(|(_, text)| text.contains(needle))
            .map(|(seq, _)| *seq)
            .collect()
    };
    for (needle, category, confidence) in [
        ("DRUGS FOR SALE", "illegal", 0.97),
        ("BUY CHEAP", "spam", 0.95),
        ("I WILL FIND YOU", "harassment", 0.9),
        ("MAYBE SPAM", "spam", 0.6),
    ] {
        let quotes = pick(needle);
        if !quotes.is_empty() {
            return verdict(true, category, confidence, quotes);
        }
    }
    verdict(false, "none", 0.9, Vec::new())
}

fn sloppy(request: &Value) -> String {
    let prompt = user_prompt(request);
    let mut quotes = target_seqs(&prompt);
    quotes.push(quotes.first().copied().unwrap_or(1) - 1);
    verdict(true, "spam", 0.99, quotes)
}

fn obedient(request: &Value) -> Option<String> {
    let prompt = user_prompt(request);
    let mut found = None;
    for line in prompt.lines() {
        let text = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|entry| entry["text"].as_str().map(str::to_owned))
            .unwrap_or_else(|| line.to_owned());
        let lower = text.to_lowercase();
        for marker in ["respond with", "reply with", "answer with"] {
            if let Some(at) = lower.find(marker) {
                found = Some(text[at + marker.len()..].trim().to_owned());
            }
        }
    }
    let ordered = found?;
    let ordered = ordered.trim_start_matches(':').trim();
    Some(
        match (ordered.find('{'), ordered.rfind('}')) {
            (Some(open), Some(close)) if open < close => &ordered[open..=close],
            _ => ordered,
        }
        .to_owned(),
    )
}
