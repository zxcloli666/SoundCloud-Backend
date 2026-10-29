use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::routing::{get, post};
use axum::{Json, Router};
use backend_contracts::diag::{Bundle, EventsBody};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::admin::DiagAuth;
use crate::error::AppError;
use crate::state::AppState;

const MAX_HOURS: i32 = 720;
const MAX_ISSUES: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/diag/bundles", post(post_bundle))
        .route("/diag/events", post(post_events))
        .route("/admin/diag/bundles/{code}", get(get_bundle))
        .route("/admin/diag/traces/{trace}", get(get_trace))
        .route("/admin/diag/issues", get(list_issues))
        .route("/admin/diag/issues/{fingerprint}", get(get_issue))
}

#[derive(Deserialize)]
struct IssuesQuery {
    hours: Option<i32>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct IssueQuery {
    hours: Option<i32>,
}

fn client_ip(parts: &Parts) -> IpAddr {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connection| connection.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

fn too_many() -> AppError {
    AppError::coded(
        StatusCode::TOO_MANY_REQUESTS,
        "diag_rate_limited",
        "Too many reports from this address, try later",
    )
    .with_retry_after(3600)
}

async fn post_bundle(
    State(state): State<AppState>,
    parts: Parts,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), AppError> {
    if !state.diag.allow_bundle(client_ip(&parts)) {
        return Err(too_many());
    }
    let bundle: Bundle = serde_json::from_slice(&body)
        .map_err(|error| AppError::bad_request(format!("bad bundle: {error}")))?;
    bundle.check().map_err(AppError::bad_request)?;
    let (code, expires) = state.diag.store_bundle(&bundle).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "code": code, "expires": expires })),
    ))
}

async fn post_events(
    State(state): State<AppState>,
    parts: Parts,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let events: EventsBody = serde_json::from_slice(&body)
        .map_err(|error| AppError::bad_request(format!("bad events: {error}")))?;
    events.check().map_err(AppError::bad_request)?;
    if !state
        .diag
        .allow_events(client_ip(&parts), events.events.len() as u32)
    {
        return Err(too_many());
    }
    state.diag.store_events(&events).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn get_bundle(
    _auth: DiagAuth,
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<Value>, AppError> {
    let bundle = state.diag.bundle(&code).await?;
    bundle
        .map(Json)
        .ok_or_else(|| AppError::not_found("no such bundle"))
}

async fn get_trace(
    _auth: DiagAuth,
    State(state): State<AppState>,
    Path(trace): Path<String>,
) -> Result<Json<Value>, AppError> {
    let trace = trace.to_ascii_lowercase();
    if trace.len() != 32 || !trace.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::bad_request("trace must be 32 hex characters"));
    }
    let view = state
        .diag
        .trace(&trace)
        .await?
        .ok_or_else(|| AppError::not_found("trace not kept or never arrived"))?;
    Ok(Json(json!(view)))
}

async fn list_issues(
    _auth: DiagAuth,
    State(state): State<AppState>,
    Query(query): Query<IssuesQuery>,
) -> Result<Json<Value>, AppError> {
    let hours = query.hours.unwrap_or(24).clamp(1, MAX_HOURS);
    let limit = query.limit.unwrap_or(50).clamp(1, MAX_ISSUES);
    let issues = state.diag.issues(hours, limit).await?;
    Ok(Json(json!(issues)))
}

async fn get_issue(
    _auth: DiagAuth,
    State(state): State<AppState>,
    Path(fingerprint): Path<String>,
    Query(query): Query<IssueQuery>,
) -> Result<Json<Value>, AppError> {
    let hours = query.hours.unwrap_or(168).clamp(1, MAX_HOURS);
    let issue = state
        .diag
        .issue(&fingerprint, hours)
        .await?
        .ok_or_else(|| AppError::not_found("no such issue in this window"))?;
    Ok(Json(json!(issue)))
}
