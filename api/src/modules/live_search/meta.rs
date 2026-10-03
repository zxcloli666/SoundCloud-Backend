use axum::Json;
use axum::http::{HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;

use crate::cache::ListPageResult;

pub const STATE_HEADER: &str = "x-search-live";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    Fresh,
    Cached,
    Stale,
    Local,
    Limited,
    Busy,
    Paused,
    Cooling,
    Unavailable,
    Timeout,
    Off,
    Skipped,
}

impl LiveState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Cached => "cached",
            Self::Stale => "stale",
            Self::Local => "local",
            Self::Limited => "limited",
            Self::Busy => "busy",
            Self::Paused => "paused",
            Self::Cooling => "cooling",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Off => "off",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LiveMeta {
    pub state: LiveState,
    pub retry_after_sec: Option<i64>,
    pub local: Option<&'static str>,
}

impl LiveMeta {
    pub fn new(state: LiveState, retry_after_sec: Option<i64>) -> Self {
        Self {
            state,
            retry_after_sec,
            local: None,
        }
    }

    pub fn local_unavailable(mut self) -> Self {
        self.local = Some("unavailable");
        self
    }
}

#[derive(Debug, Serialize)]
pub struct LivePage {
    #[serde(flatten)]
    pub page: ListPageResult<Value>,
    pub live: LiveMeta,
    pub weak: bool,
}

impl LivePage {
    pub fn new(page: ListPageResult<Value>, live: LiveMeta) -> Self {
        Self {
            page,
            live,
            weak: false,
        }
    }

    pub fn state(&self) -> LiveState {
        self.live.state
    }
}

impl IntoResponse for LivePage {
    fn into_response(self) -> Response {
        let state = self.live.state.as_str();
        let mut response = Json(self).into_response();
        response.headers_mut().insert(
            HeaderName::from_static(STATE_HEADER),
            HeaderValue::from_static(state),
        );
        response
    }
}
