use axum::http::StatusCode;
use axum::response::Response;
use serde_json::Value;

use crate::cache::cache_service::CacheScope;
use crate::common::response::json_response;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub struct CacheOpts<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub ttl_sec: u64,
    pub cache_key: Option<&'a str>,
}

pub async fn cached_or_fetch<F, Fut>(
    st: &AppState,
    opts: CacheOpts<'_>,
    fetch: F,
) -> AppResult<Response>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = AppResult<Value>>,
{
    let key = st
        .cache
        .build_key(opts.method, opts.url, CacheScope::Shared, None);
    if let Ok(Some(raw)) = st.cache.get_raw(&key).await {
        return Ok(json_response(StatusCode::OK, raw));
    }
    let value = fetch().await?;
    let payload = serde_json::to_string(&value)
        .map_err(|error| AppError::internal(format!("json encode: {error}")))?;
    let _ = st
        .cache
        .set_raw(
            &key,
            &payload,
            opts.ttl_sec,
            opts.cache_key,
            CacheScope::Shared,
            None,
        )
        .await;
    Ok(json_response(StatusCode::OK, payload))
}
