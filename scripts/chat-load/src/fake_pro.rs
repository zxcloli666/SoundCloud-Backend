use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde::Deserialize;
use uuid::Uuid;

use crate::b64;
use crate::pro::{END, Open, Opened};

pub const ORIGIN_TIMEOUT: Duration = Duration::from_secs(60);
pub const TOTAL: &str = "x-pro-total";

#[derive(Clone)]
struct Pro {
    origin: reqwest::Client,
    answers: Arc<Mutex<HashMap<String, Bytes>>>,
}

#[derive(Deserialize)]
struct Take {
    #[serde(default)]
    from: usize,
    max: Option<usize>,
}

pub fn router() -> anyhow::Result<Router> {
    let pro = Pro {
        origin: reqwest::Client::builder()
            .no_proxy()
            .timeout(ORIGIN_TIMEOUT)
            .build()?,
        answers: Arc::new(Mutex::new(HashMap::new())),
    };
    Ok(Router::new()
        .route("/open", post(open))
        .route("/take/{id}", get(take).delete(close))
        .route("/health", get(|| async { "ok" }))
        .with_state(pro))
}

pub async fn serve(bind: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("fake relay-pro on {bind}");
    axum::serve(listener, router()?).await?;
    Ok(())
}

async fn open(State(pro): State<Pro>, Json(asked): Json<Open>) -> Response {
    let Ok(method) = Method::from_bytes(asked.method.as_bytes()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut request = pro.origin.request(method, &asked.url);
    for (name, value) in &asked.headers {
        request = request.header(name, value);
    }
    if let Some(body) = asked.body.as_deref().and_then(b64::decode) {
        request = request.body(body);
    }
    let Ok(answer) = request.send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let status = answer.status().as_u16();
    let headers = answer
        .headers()
        .iter()
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
        .collect();
    let Ok(body) = answer.bytes().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let id = Uuid::new_v4().simple().to_string();
    if let Ok(mut answers) = pro.answers.lock() {
        answers.insert(id.clone(), body);
    }
    Json(Opened {
        id,
        status,
        headers,
    })
    .into_response()
}

async fn take(
    State(pro): State<Pro>,
    Path(id): Path<String>,
    Query(take): Query<Take>,
) -> Response {
    let body = pro
        .answers
        .lock()
        .ok()
        .and_then(|answers| answers.get(&id).cloned());
    let Some(body) = body else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let from = take.from.min(body.len());
    let to = take
        .max
        .map_or(body.len(), |max| (from + max).min(body.len()));
    let end = if to == body.len() { "1" } else { "0" };
    let mut response = body.slice(from..to).into_response();
    let headers = response.headers_mut();
    headers.insert(HeaderName::from_static(END), HeaderValue::from_static(end));
    if let Ok(total) = HeaderValue::from_str(&body.len().to_string()) {
        headers.insert(HeaderName::from_static(TOTAL), total);
    }
    response
}

async fn close(State(pro): State<Pro>, Path(id): Path<String>) -> StatusCode {
    if let Ok(mut answers) = pro.answers.lock() {
        answers.remove(&id);
    }
    StatusCode::NO_CONTENT
}
