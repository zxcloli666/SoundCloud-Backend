use std::collections::HashMap;
use std::future::pending;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use futures::stream;
use llm_ladder::{Observer, StepReport, Usage};
use serde_json::{Value, json};
use tokio::time::sleep;

pub const MESSAGES: &str = "/v1/messages";
pub const COMPLETIONS: &str = "/v1/chat/completions";

const FLUSH_BEFORE_BREAK: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub enum Reply {
    Events(Vec<(u64, String)>),
    Stalls(Vec<(u64, String)>),
    Breaks(Vec<(u64, String)>),
    Whole(u64, Value),
    Status(u16, Value),
}

#[derive(Clone)]
pub struct Call {
    pub path: String,
    pub headers: HeaderMap,
    pub body: String,
}

pub struct Vendor {
    address: SocketAddr,
    shared: Arc<Shared>,
}

#[derive(Default)]
pub struct Seen(Mutex<Vec<Step>>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub provider: String,
    pub result: &'static str,
    pub usage: Usage,
    pub cost_micro_usd: u64,
}

#[derive(Default)]
struct Shared {
    replies: Mutex<HashMap<String, Reply>>,
    calls: Mutex<Vec<Call>>,
    hung_up: AtomicUsize,
}

struct Line {
    shared: Arc<Shared>,
    finished: bool,
}

impl Vendor {
    pub async fn open() -> Self {
        let shared = Arc::new(Shared::default());
        let app = Router::new()
            .route(MESSAGES, post(answer))
            .route(COMPLETIONS, post(answer))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free local port");
        let address = listener.local_addr().expect("the bound address");
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { address, shared }
    }

    pub fn model(&self, model: &str, reply: Reply) {
        self.shared
            .replies
            .lock()
            .unwrap()
            .insert(model.to_owned(), reply);
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    pub fn calls(&self) -> Vec<Call> {
        self.shared.calls.lock().unwrap().clone()
    }

    pub fn calls_of(&self, model: &str) -> Vec<Call> {
        let marker = format!("\"model\":\"{model}\"");
        self.calls()
            .into_iter()
            .filter(|call| call.body.contains(&marker))
            .collect()
    }

    pub async fn hung_up(&self, times: usize) -> bool {
        let limit = Instant::now() + Duration::from_secs(10);
        while self.shared.hung_up.load(Ordering::SeqCst) < times {
            if Instant::now() > limit {
                return false;
            }
            sleep(Duration::from_millis(20)).await;
        }
        true
    }
}

impl Seen {
    pub fn steps(&self) -> Vec<Step> {
        self.0.lock().unwrap().clone()
    }

    pub fn results(&self) -> Vec<(String, &'static str)> {
        self.steps()
            .into_iter()
            .map(|step| (step.provider, step.result))
            .collect()
    }
}

impl Observer for Seen {
    fn step(&self, report: &StepReport<'_>) {
        self.0.lock().unwrap().push(Step {
            provider: report.provider.to_owned(),
            result: report.result.as_str(),
            usage: report.usage,
            cost_micro_usd: report.cost_micro_usd,
        });
    }
}

impl Call {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("the request body is JSON")
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

impl Line {
    fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        if !self.finished {
            self.shared.hung_up.fetch_add(1, Ordering::SeqCst);
        }
    }
}

pub fn message_events(
    pieces: &[&str],
    input_tokens: u64,
    output_tokens: u64,
) -> Vec<(u64, String)> {
    let start = json!({"input_tokens": input_tokens, "output_tokens": 1});
    message_events_counting(pieces, start, output_tokens)
}

pub fn message_events_counting(
    pieces: &[&str],
    start: Value,
    output_tokens: u64,
) -> Vec<(u64, String)> {
    let mut events = vec![
        event(
            0,
            "message_start",
            json!({"type": "message_start", "message": {"usage": start}}),
        ),
        event(
            0,
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        ),
        event(0, "ping", json!({"type": "ping"})),
    ];
    events.extend(pieces.iter().map(|piece| {
        event(150, "content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": piece}}))
    }));
    events.extend([
        event(0, "content_block_stop", json!({"type": "content_block_stop", "index": 0})),
        event(0, "message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": output_tokens}})),
        event(0, "message_stop", json!({"type": "message_stop"})),
    ]);
    events
}

pub fn whole_message(text: &str, input_tokens: u64, output_tokens: u64) -> Value {
    json!({
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens},
    })
}

pub fn completion_chunks(pieces: &[&str], usage: Option<(u64, u64)>) -> Vec<(u64, String)> {
    let mut chunks = vec![chunk(
        0,
        json!({"choices": [{"index": 0, "delta": {"role": "assistant", "reasoning_content": "thinking it over"}}], "usage": null}),
    )];
    chunks.extend(pieces.iter().map(|piece| {
        chunk(
            150,
            json!({"choices": [{"index": 0, "delta": {"content": piece}}], "usage": null}),
        )
    }));
    if let Some((prompt_tokens, completion_tokens)) = usage {
        chunks.push(chunk(0, json!({"choices": [], "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens}})));
    }
    chunks.push((0, "data: [DONE]\n\n".to_owned()));
    chunks
}

pub fn whole_completion(text: &str, usage: Option<(u64, u64)>) -> Value {
    let usage = usage.map(|(prompt_tokens, completion_tokens)| {
        json!({"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens})
    });
    json!({
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text, "reasoning_content": "thinking it over"}, "finish_reason": "stop"}],
        "usage": usage,
    })
}

async fn answer(
    State(shared): State<Arc<Shared>>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let model = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|body| body["model"].as_str().map(str::to_owned))
        .unwrap_or_default();
    shared.calls.lock().unwrap().push(Call {
        path: uri.path().to_owned(),
        headers,
        body,
    });
    let reply = shared.replies.lock().unwrap().get(&model).cloned();
    let line = Line {
        shared,
        finished: false,
    };
    match reply {
        Some(Reply::Events(chunks)) => streamed(chunks, line, End::Closes),
        Some(Reply::Stalls(chunks)) => streamed(chunks, line, End::Stalls),
        Some(Reply::Breaks(chunks)) => streamed(chunks, line, End::Breaks),
        Some(Reply::Whole(delay, body)) => {
            sleep(Duration::from_millis(delay)).await;
            line.finish();
            axum::Json(body).into_response()
        }
        Some(Reply::Status(status, body)) => {
            line.finish();
            let status = StatusCode::from_u16(status).expect("an http status");
            (status, axum::Json(body)).into_response()
        }
        None => {
            line.finish();
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

#[derive(Clone, Copy)]
enum End {
    Closes,
    Stalls,
    Breaks,
}

fn streamed(chunks: Vec<(u64, String)>, line: Line, end: End) -> Response {
    let state = (chunks.into_iter(), Some(line));
    let body = stream::unfold(state, move |(mut chunks, mut line)| async move {
        if let Some((delay, chunk)) = chunks.next() {
            sleep(Duration::from_millis(delay)).await;
            return Some((Ok(Bytes::from(chunk)), (chunks, line)));
        }
        let line = line.take()?;
        match end {
            End::Closes => {
                line.finish();
                None
            }
            End::Stalls => pending().await,
            End::Breaks => {
                line.finish();
                sleep(FLUSH_BEFORE_BREAK).await;
                let cut = std::io::Error::other("the vendor cut the line");
                Some((Err(cut), (chunks, None)))
            }
        }
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body),
    )
        .into_response()
}

fn event(delay: u64, kind: &str, data: Value) -> (u64, String) {
    (delay, format!("event: {kind}\ndata: {data}\n\n"))
}

fn chunk(delay: u64, data: Value) -> (u64, String) {
    (delay, format!("data: {data}\n\n"))
}
