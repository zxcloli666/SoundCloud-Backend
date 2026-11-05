use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::modules::auth::sealed::dto::{
    AuthResult, LinkClaimBody, LinkClaimed, LinkCreateBody, LinkCreated, LinkPoll, RequestId,
};
use crate::modules::auth::sealed::signed::SealedAuth;
use crate::modules::auth::service::login::{LoginInitResult, LoginStatusResult};
use crate::modules::auth::{AuthService, LinkService};
use crate::modules::chat::body::{ChatJson, ChatQuery};

#[derive(Clone)]
struct Services {
    auth: Arc<AuthService>,
    link: Arc<LinkService>,
}

pub fn inner(auth: Arc<AuthService>, link: Arc<LinkService>) -> Router {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/login/status", get(login_status))
        .route("/auth/link/create", post(link_create))
        .route("/auth/link/claim", post(link_claim))
        .route("/auth/link/status", get(link_status))
        .with_state(Services { auth, link })
}

async fn login(auth: SealedAuth, State(services): State<Services>) -> AuthResult<LoginInitResult> {
    let started = services
        .auth
        .initiate_bound_login(auth.session_id, &auth.bind)
        .await?;
    Ok(Json(started))
}

async fn login_status(
    auth: SealedAuth,
    State(services): State<Services>,
    ChatQuery(request): ChatQuery<RequestId>,
) -> AuthResult<LoginStatusResult> {
    let status = services
        .auth
        .login_request_status_for(request.id, |stored| auth.keys.owns(stored))
        .await?;
    Ok(Json(status))
}

async fn link_create(
    auth: SealedAuth,
    State(services): State<Services>,
    ChatJson(body): ChatJson<LinkCreateBody>,
) -> AuthResult<LinkCreated> {
    let source = auth.session_id.filter(|_| body.mode != "pull");
    let created = services
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

async fn link_claim(
    auth: SealedAuth,
    State(services): State<Services>,
    ChatJson(body): ChatJson<LinkClaimBody>,
) -> AuthResult<LinkClaimed> {
    let claimed = services
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

async fn link_status(
    auth: SealedAuth,
    State(services): State<Services>,
    ChatQuery(request): ChatQuery<RequestId>,
) -> AuthResult<LinkPoll> {
    let status = services
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
