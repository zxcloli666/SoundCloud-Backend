use std::sync::Arc;
use std::time::{Duration, Instant};

use async_nats::jetstream::stream;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use backend_contracts::pipeline::REPLY_TO_HEADER;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::worker::WorkerRpc;
use crate::bus::nats::NatsService;
use crate::config::{LlmCfg, VoiceChatCfg};

pub const CHAT_SECTION: &str = r#"
[voice_chat]
ladder = "chat"
requests_per_minute = 12
max_text_chars = 40
max_history = 6
max_notes = 3
max_tools = 8
max_tool_rounds = 3
max_system_bytes = 16000
instructions = "Keep it short."

[voice_chat.personas.default]
prompt = "A friendly voice."

[voice_chat.personas.loli]
prompt = "A cheerful voice named Loli."
"#;

const LADDER: &str = r#"
[providers.paid]
kind = "anthropic_compatible"
endpoint = "https://llm.example/v1/messages"
model = "example-model"
api_key = "env:EXAMPLE_KEY"
max_output_tokens = 600
total_timeout_ms = 12000

[ladders.chat]
steps = ["paid"]
hedge_after_ms = 2000
deadline_ms = 16000
breaker = { failures = 3, window_s = 30, open_s = 90 }
"#;

pub fn limits() -> VoiceChatCfg {
    LlmCfg::parse(&format!("{LADDER}{CHAT_SECTION}"))
        .expect("a valid config")
        .voice_chat
        .expect("the chat section")
}

pub fn body_limit() -> usize {
    super::request::body_limit(&limits())
}

pub fn tools() -> Value {
    json!([
        {
            "name": "volume_set",
            "description": "Set the playback volume.",
            "args": {
                "type": "object",
                "properties": {"level": {"type": "integer", "minimum": 0, "maximum": 100}},
                "required": ["level"],
            },
        },
        {
            "name": "search_tracks",
            "description": "Find tracks by name.",
            "args": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
            "returns": true,
        },
    ])
}

pub struct Heard {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub events: Vec<(Duration, String, Value)>,
    pub body: Value,
}

impl Heard {
    pub fn named(&self) -> Vec<(&str, &Value)> {
        self.events
            .iter()
            .map(|(_, name, data)| (name.as_str(), data))
            .collect()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

pub async fn hear(response: Response) -> anyhow::Result<Heard> {
    let started = Instant::now();
    let status = response.status();
    let headers = response.headers().clone();
    let mut frames = response.into_body().into_data_stream();
    let mut whole = Vec::new();
    let mut pending = Vec::new();
    let mut events = Vec::new();
    while let Some(chunk) = frames.next().await {
        let chunk = chunk?;
        whole.extend_from_slice(&chunk);
        pending.extend_from_slice(&chunk);
        while let Some(frame) = next_frame(&mut pending) {
            events.extend(event(&frame).map(|(name, data)| (started.elapsed(), name, data)));
        }
    }
    Ok(Heard {
        status,
        headers,
        events,
        body: serde_json::from_slice(&whole).unwrap_or(Value::Null),
    })
}

pub struct Listening {
    frames: axum::body::BodyDataStream,
    pending: Vec<u8>,
}

impl Listening {
    pub fn to(response: Response) -> Self {
        Self {
            frames: response.into_body().into_data_stream(),
            pending: Vec::new(),
        }
    }

    pub async fn next(&mut self) -> Option<(String, Value)> {
        loop {
            if let Some(frame) = next_frame(&mut self.pending) {
                match event(&frame) {
                    Some(event) => return Some(event),
                    None => continue,
                }
            }
            let chunk = self.frames.next().await?.ok()?;
            self.pending.extend_from_slice(&chunk);
        }
    }
}

pub struct Lane {
    pub rpc: Arc<WorkerRpc>,
    pub nats: Arc<NatsService>,
    pub subject: String,
    stream: String,
}

pub fn nats_url() -> String {
    std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned())
}

pub async fn lane() -> Lane {
    let nats = NatsService::connect(&nats_url(), CancellationToken::new())
        .await
        .expect("nats");
    let id = Uuid::now_v7().simple().to_string();
    let stream = format!("LLM_TEST_{id}");
    nats.jetstream()
        .create_stream(stream::Config {
            name: stream.clone(),
            subjects: vec![format!("llmtest.{id}.>")],
            storage: stream::StorageType::Memory,
            max_age: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("a test stream");
    Lane {
        rpc: WorkerRpc::new(nats.clone()),
        nats,
        subject: format!("llmtest.{id}.complete"),
        stream,
    }
}

impl Lane {
    pub async fn worker(
        &self,
        reply: Option<Value>,
    ) -> tokio::task::JoinHandle<async_nats::Message> {
        let mut calls = self
            .nats
            .subscribe(&self.subject)
            .await
            .expect("a subscription");
        let nats = self.nats.clone();
        tokio::spawn(async move {
            let call = calls.next().await.expect("a call");
            let inbox = call
                .headers
                .as_ref()
                .and_then(|headers| headers.get(REPLY_TO_HEADER))
                .expect("a reply inbox")
                .to_string();
            if let Some(reply) = reply {
                nats.publish_core(&inbox, reply.to_string().into())
                    .await
                    .expect("the reply is sent");
            }
            call
        })
    }

    pub async fn close(self) {
        self.nats
            .jetstream()
            .delete_stream(&self.stream)
            .await
            .expect("the stream is removed");
    }
}

fn next_frame(pending: &mut Vec<u8>) -> Option<String> {
    let end = pending.windows(2).position(|pair| pair == b"\n\n")?;
    let frame: Vec<u8> = pending.drain(..end + 2).collect();
    Some(String::from_utf8_lossy(&frame).into_owned())
}

fn event(frame: &str) -> Option<(String, Value)> {
    let field = |name: &str| {
        frame
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|value| value.trim().to_owned())
    };
    let data = serde_json::from_str(&field("data:")?).ok()?;
    Some((field("event:")?, data))
}
