use std::net::IpAddr;

use axum::body::Body;
use axum::http::{Extensions, Request};
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::{Inner, InnerResponse};
use sc_chat_wire::seal::OpenedRequest;

use crate::modules::auth::sealed::Sealed;
use crate::modules::auth::sealed::claims::{ENC_TTL, claim, enc_key};
use crate::modules::auth::sealed::decide::{poll, start};
use crate::modules::auth::sealed::kind::Kind;
use crate::modules::auth::sealed::metrics::note;
use crate::modules::auth::sealed::refusal::Refusal;
use crate::modules::chat::ChatError;
use crate::modules::chat::channel::clock;
use crate::modules::chat::channel::dispatch::{dispatch, inner_request};
use crate::modules::chat::channel::sealed_auth::SealedAuth;

pub async fn answer(
    sealed: &Sealed,
    opened: &OpenedRequest,
    client: Option<IpAddr>,
) -> InnerResponse {
    match prepare(sealed, opened, client).await {
        Ok(request) => dispatch(&sealed.inner, request).await,
        Err(refusal) => refusal.into_answer(),
    }
}

async fn prepare(
    sealed: &Sealed,
    opened: &OpenedRequest,
    client: Option<IpAddr>,
) -> Result<Request<Body>, Refusal> {
    let inner = Inner::decode(opened.inner.as_slice()).map_err(|_| ChatError::Invalid)?;
    let kind = Kind::parse(&inner.path).ok_or(ChatError::NotFound)?;
    if sealed.cfg.plain && kind.starts() {
        return Err(ChatError::Disabled.into());
    }
    clock::check(inner.ts_ms)?;
    let auth = SealedAuth::verify(&inner, &opened.enc)?;
    admit(sealed, kind, &inner, client).await?;
    claim(&sealed.redis, enc_key(&opened.enc), ENC_TTL)
        .await
        .inspect_err(|_| note(kind.path(), "none", "replay"))?;
    let mut extensions = Extensions::new();
    extensions.insert(auth);
    Ok(inner_request(inner, extensions)?)
}

async fn admit(
    sealed: &Sealed,
    kind: Kind,
    inner: &Inner,
    client: Option<IpAddr>,
) -> Result<(), Refusal> {
    let client = client.ok_or(ChatError::Unavailable)?;
    let gates = &sealed.gates;
    let path = kind.path();
    let auth_pub = inner
        .bind
        .as_ref()
        .map_or(&[][..], |bind| bind.auth_pub.as_slice());
    match kind {
        Kind::Login => {
            start(
                sealed,
                &gates.login,
                path,
                inner.pow.as_ref(),
                auth_pub,
                client,
            )
            .await
        }
        Kind::LinkCreate => {
            start(
                sealed,
                &gates.link,
                path,
                inner.pow.as_ref(),
                auth_pub,
                client,
            )
            .await
        }
        Kind::LoginStatus => poll(&gates.login_status, path, client),
        Kind::LinkStatus => poll(&gates.link_status, path, client),
        Kind::LinkClaim => poll(&gates.link_claim, path, client),
    }
}
