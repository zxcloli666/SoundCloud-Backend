use std::time::Duration;

use axum::Router;
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;

use crate::common::admin::AdminAuth;
use crate::state::AppState;

const DEFAULT_SECONDS: u64 = 20;
const MAX_SECONDS: u64 = 120;
const DEFAULT_HERTZ: i32 = 199;
const MAX_HERTZ: i32 = 999;

#[derive(Deserialize)]
struct ProfileQuery {
    seconds: Option<u64>,
    hertz: Option<i32>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/profile", get(flamegraph))
}

async fn flamegraph(_: AdminAuth, Query(query): Query<ProfileQuery>) -> Response {
    let seconds = query
        .seconds
        .unwrap_or(DEFAULT_SECONDS)
        .clamp(1, MAX_SECONDS);
    let hertz = query.hertz.unwrap_or(DEFAULT_HERTZ).clamp(1, MAX_HERTZ);

    let guard = match pprof::ProfilerGuardBuilder::default()
        .frequency(hertz)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
    {
        Ok(guard) => guard,
        Err(error) => return failed(format!("profiler could not start: {error}")),
    };

    tokio::time::sleep(Duration::from_secs(seconds)).await;

    let report = match guard.report().build() {
        Ok(report) => report,
        Err(error) => return failed(format!("profile could not be collected: {error}")),
    };

    let mut svg = Vec::new();
    if let Err(error) = report.flamegraph(&mut svg) {
        return failed(format!("flamegraph could not be drawn: {error}"));
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}

fn failed(message: String) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
}
