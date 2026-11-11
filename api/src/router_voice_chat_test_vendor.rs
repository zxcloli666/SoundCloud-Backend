use std::collections::HashMap;
use std::future::pending;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use futures::stream;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time::sleep;

pub const MESSAGES: &str = "/v1/messages";
pub const COMPLETIONS: &str = "/v1/chat/completions";

const FLUSH_BEFORE_BREAK: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub enum Reply {
    Stream(Vec<(u64, String)>, End),
    Gated(Vec<(u64, String)>, usize),
    Status(u16),
}

#[derive(Clone, Copy)]
pub enum End {
    Closes,
    Stalls,
    Breaks,
    Gated(usize),
}

#[derive(Clone)]
pub struct Call {
    pub headers: HeaderMap,
    pub body: Value,
}

pub struct Vendor {
    address: SocketAddr,
    shared: Arc<Shared>,
}

struct Shared {
    replies: Mutex<HashMap<String, Reply>>,
    calls: Mutex<Vec<Call>>,
    hung_up: AtomicUsize,
    gate: Semaphore,
}

struct Line {
    shared: Arc<Shared>,
    finished: bool,
}

impl Vendor {
    pub async fn open() -> anyhow::Result<Self> {
        let shared = Arc::new(Shared {
            replies: Mutex::default(),
            calls: Mutex::default(),
            hung_up: AtomicUsize::new(0),
            gate: Semaphore::new(0),
        });
        let app = Router::new()
            .route(MESSAGES, post(answer))
            .route(COMPLETIONS, post(answer))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { address, shared })
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

    pub fn release(&self) {
        self.shared.gate.add_permits(1);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.shared.calls.lock().unwrap().clone()
    }

    pub fn calls_of(&self, model: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.body["model"] == model)
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

pub fn message(
    pieces: &[&str],
    gap_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
) -> Vec<(u64, String)> {
    let mut events = vec![event(
        0,
        "message_start",
        json!({"type": "message_start", "message": {"usage": {"input_tokens": input_tokens, "output_tokens": 1}}}),
    )];
    events.extend(pieces.iter().map(|piece| {
        event(
            gap_ms,
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": piece}}),
        )
    }));
    events.extend([
        event(
            0,
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": output_tokens}}),
        ),
        event(0, "message_stop", json!({"type": "message_stop"})),
    ]);
    events
}

pub fn completion(pieces: &[&str], gap_ms: u64) -> Vec<(u64, String)> {
    let mut chunks: Vec<(u64, String)> = pieces
        .iter()
        .map(|piece| {
            let data =
                json!({"choices": [{"index": 0, "delta": {"content": piece}}], "usage": null});
            (gap_ms, format!("data: {data}\n\n"))
        })
        .collect();
    chunks.push((0, "data: [DONE]\n\n".to_owned()));
    chunks
}

async fn answer(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: String) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let model = body["model"].as_str().unwrap_or_default().to_owned();
    shared.calls.lock().unwrap().push(Call { headers, body });
    let reply = shared.replies.lock().unwrap().get(&model).cloned();
    let line = Line {
        shared,
        finished: false,
    };
    match reply {
        Some(Reply::Stream(chunks, end)) => streamed(chunks, line, end),
        Some(Reply::Gated(chunks, before)) => streamed(chunks, line, End::Gated(before)),
        Some(Reply::Status(status)) => {
            line.finish();
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            (
                status,
                axum::Json(json!({"error": {"type": "overloaded_error"}})),
            )
                .into_response()
        }
        None => {
            line.finish();
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

fn streamed(chunks: Vec<(u64, String)>, line: Line, end: End) -> Response {
    let state = (chunks.into_iter().enumerate(), Some(line));
    let body = stream::unfold(state, move |(mut chunks, mut line)| async move {
        if let Some((index, (delay, chunk))) = chunks.next() {
            if let (End::Gated(before), Some(line)) = (end, &line)
                && index == before
                && let Ok(permit) = line.shared.gate.acquire().await
            {
                permit.forget();
            }
            sleep(Duration::from_millis(delay)).await;
            return Some((Ok(Bytes::from(chunk)), (chunks, line)));
        }
        let line = line.take()?;
        match end {
            End::Closes | End::Gated(_) => {
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
