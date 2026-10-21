use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chat_wire::prost::Message;
use chat_wire::proto::{Header, InnerResponse};
use chat_wire::seal::OpenedRequest;

use crate::modules::chat::ChatError;

const BUSY_RETRY_AFTER_S: &str = "1";

pub fn error_answer(error: ChatError) -> InnerResponse {
    InnerResponse {
        status: u32::from(error.status().as_u16()),
        headers: vec![Header {
            name: CONTENT_TYPE.as_str().to_owned(),
            value: "application/json".to_owned(),
        }],
        body: error.body().to_string().into_bytes(),
    }
}

pub fn sealed(opened: &OpenedRequest, answer: InnerResponse) -> Response {
    let Ok(nonce) = chat_wire::random_bytes() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(body) = opened
        .response_key
        .seal(&opened.enc, nonce, &answer.encode_to_vec())
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub fn bad_envelope() -> Response {
    (StatusCode::BAD_REQUEST, "bad envelope").into_response()
}

pub fn too_large() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "envelope too large").into_response()
}

pub fn busy() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(RETRY_AFTER, BUSY_RETRY_AFTER_S)],
        "busy",
    )
        .into_response()
}
