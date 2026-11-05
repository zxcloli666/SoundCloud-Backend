use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::response::Response;
use sc_chat_wire::seal::{MAX_ENVELOPE, ock_version, open_request};

use crate::modules::chat::channel::Channel;
use crate::modules::chat::channel::answer::{bad_envelope, busy, sealed, too_large};
use crate::modules::chat::channel::prepare::answer;

pub async fn exchange(State(channel): State<Arc<Channel>>, request: Request) -> Response {
    if declared_length(&request).is_some_and(|length| length > MAX_ENVELOPE) {
        return unsealed("too_large", too_large());
    }
    let Ok(_permit) = channel.busy.try_acquire() else {
        return busy();
    };
    let Ok(body) = to_bytes(request.into_body(), MAX_ENVELOPE).await else {
        return unsealed("too_large", too_large());
    };
    let Ok(version) = ock_version(&body) else {
        return unsealed("header", bad_envelope());
    };
    let Some(secret) = channel.chat.ocks.get(version) else {
        return unsealed("ock_version", bad_envelope());
    };
    let Ok(opened) = open_request(&body, secret) else {
        return unsealed("open", bad_envelope());
    };
    drop(body);
    let inner_answer = answer(&channel, &opened).await;
    sealed(&opened, inner_answer)
}

fn unsealed(reason: &'static str, answer: Response) -> Response {
    metrics::counter!("chat_unseal_failures_total", "reason" => reason).increment(1);
    answer
}

fn declared_length(request: &Request) -> Option<usize> {
    request
        .headers()
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}
