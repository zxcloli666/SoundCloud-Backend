use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::modules::auth::sealed::dto::{
    AuthResult, LinkClaimBody, LinkClaimed, LinkCreateBody, LinkCreated, LinkPoll, RequestId,
};
use crate::modules::auth::service::login::{LoginInitResult, LoginStatusResult};
use crate::modules::chat::ChatState;
use crate::modules::chat::body::{ChatJson, ChatQuery};
use crate::modules::chat::channel::sealed_auth::SealedAuth;

pub async fn login(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
) -> AuthResult<LoginInitResult> {
    let started = chat
        .auth
        .initiate_bound_login(auth.session_id, &auth.bind)
        .await?;
    Ok(Json(started))
}

pub async fn login_status(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(request): ChatQuery<RequestId>,
) -> AuthResult<LoginStatusResult> {
    let status = chat
        .auth
        .login_request_status_for(request.id, |stored| auth.keys.owns(stored))
        .await?;
    Ok(Json(status))
}

pub async fn link_create(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatJson(body): ChatJson<LinkCreateBody>,
) -> AuthResult<LinkCreated> {
    let source = auth.session_id.filter(|_| body.mode != "pull");
    let created = chat
        .link
        .create_bound(&body.mode, source, Some(&auth.bind), |stored| {
            auth.keys.owns(stored)
        })
        .await?;
    Ok(Json(LinkCreated {
        link_request_id: created.link_request_id,
        claim_token: created.claim_token,
        expires_at: created.expires_at.and_utc().timestamp_millis(),
        mode: body.mode,
    }))
}

pub async fn link_claim(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatJson(body): ChatJson<LinkClaimBody>,
) -> AuthResult<LinkClaimed> {
    let claimed = chat
        .link
        .claim_bound(
            &body.claim_token,
            auth.session_id,
            Some(&auth.bind),
            |stored| auth.keys.owns(stored),
        )
        .await?;
    Ok(Json(LinkClaimed {
        session_id: claimed.session_id,
        mode: claimed.mode,
    }))
}

pub async fn link_status(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(request): ChatQuery<RequestId>,
) -> AuthResult<LinkPoll> {
    let status = chat
        .link
        .get_status_for(request.id, |stored| auth.keys.owns(stored))
        .await?;
    Ok(Json(LinkPoll {
        status: status.status,
        mode: status.mode,
        session_id: status.session_id,
        error: status.error,
    }))
}

#[cfg(test)]
#[path = "tests/auth_tests.rs"]
mod tests;
