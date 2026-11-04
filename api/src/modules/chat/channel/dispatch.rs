use axum::body::{Body, to_bytes};
use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
use axum::http::{Extensions, HeaderMap, HeaderName, Method, Request, StatusCode, Uri};
use axum::response::Response;
use sc_chat_wire::proto::{Header, Inner, InnerResponse};
use tower::ServiceExt;

use crate::modules::chat::channel::Channel;
use crate::modules::chat::channel::answer::error_answer;
use crate::modules::chat::sync::wire::RETRY_AFTER_MS;
use crate::modules::chat::{ChatError, ChatResult};

pub const ANSWER_MAX: usize = 1024 * 1024;

pub fn inner_request(inner: Inner, extensions: Extensions) -> ChatResult<Request<Body>> {
    let method = Method::from_bytes(inner.method.as_bytes()).map_err(|_| ChatError::Invalid)?;
    let target = match inner.query.is_empty() {
        true => inner.path,
        false => format!("{}?{}", inner.path, inner.query),
    };
    let uri = Uri::try_from(target).map_err(|_| ChatError::Invalid)?;
    let mut builder = Request::builder().method(method).uri(uri);
    if !inner.body.is_empty() {
        builder = builder.header(CONTENT_TYPE, "application/json");
    }
    let mut request = builder
        .body(Body::from(inner.body))
        .map_err(|_| ChatError::Invalid)?;
    *request.extensions_mut() = extensions;
    Ok(request)
}

pub async fn dispatch(channel: &Channel, request: Request<Body>) -> InnerResponse {
    let response = channel
        .inner
        .clone()
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {});
    inner_response(response).await
}

async fn inner_response(response: Response) -> InnerResponse {
    let (parts, body) = response.into_parts();
    let Ok(body) = to_bytes(body, ANSWER_MAX).await else {
        tracing::warn!(status = %parts.status, "a chat answer is over the answer cap");
        return error_answer(ChatError::Internal);
    };
    let unrouted = matches!(
        parts.status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    );
    if unrouted && body.is_empty() {
        return error_answer(ChatError::NotFound);
    }
    InnerResponse {
        status: u32::from(parts.status.as_u16()),
        headers: kept_headers(&parts.headers),
        body: body.to_vec(),
    }
}

fn kept_headers(headers: &HeaderMap) -> Vec<Header> {
    [
        CONTENT_TYPE,
        RETRY_AFTER,
        HeaderName::from_static(RETRY_AFTER_MS),
    ]
    .iter()
    .filter_map(|name| {
        let value = headers.get(name)?.to_str().ok()?;
        Some(Header {
            name: name.as_str().to_owned(),
            value: value.to_owned(),
        })
    })
    .collect()
}
