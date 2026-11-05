use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::response::Response;
use sc_chat_wire::seal::{MAX_ENVELOPE, ock_version, open_request};

use crate::common::admission::client_address;
use crate::modules::auth::sealed::Sealed;
use crate::modules::auth::sealed::metrics::note;
use crate::modules::auth::sealed::prepare::answer;
use crate::modules::chat::channel::answer::{bad_envelope, busy, sealed as seal, too_large};

pub async fn exchange(State(sealed): State<Arc<Sealed>>, request: Request) -> Response {
    if declared_length(&request).is_some_and(|length| length > MAX_ENVELOPE) {
        return unsealed("too_large", too_large());
    }
    let Ok(_permit) = sealed.busy.try_acquire() else {
        return busy();
    };
    let client = client(&sealed, &request);
    let Ok(body) = to_bytes(request.into_body(), MAX_ENVELOPE).await else {
        return unsealed("too_large", too_large());
    };
    let Ok(version) = ock_version(&body) else {
        return unsealed("header", bad_envelope());
    };
    let Some(secret) = sealed.ocks.get(version) else {
        return unsealed("ock_version", bad_envelope());
    };
    let Ok(opened) = open_request(&body, secret) else {
        return unsealed("open", bad_envelope());
    };
    drop(body);
    let inner_answer = answer(&sealed, &opened, client).await;
    seal(&opened, inner_answer)
}

fn unsealed(reason: &'static str, answer: Response) -> Response {
    note("unseal", "none", reason);
    answer
}

fn client(sealed: &Sealed, request: &Request) -> Option<IpAddr> {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()?
        .0
        .ip();
    Some(client_address(
        peer,
        request.headers(),
        &sealed.cfg.trusted_proxies,
    ))
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
