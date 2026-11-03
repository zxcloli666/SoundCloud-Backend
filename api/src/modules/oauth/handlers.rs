use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, LOCATION, PRAGMA};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use backend_contracts::remote::LinkProvider;
use serde_json::json;

use super::page::{self, PageFields};
use super::request::{
    AuthorizeForm, AuthorizeParams, TokenForm, basic_credentials, known_target, redirect_with_code,
};
use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::modules::remote::secrets::{normalize_code, secret_matches};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/oauth/authorize",
            get(authorize_page).post(authorize_submit),
        )
        .route("/oauth/token", post(token))
}

fn html(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (PRAGMA, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}

fn refused(status: StatusCode, text: &str) -> Response {
    html(status, page::failure(text))
}

fn target_name(provider: LinkProvider) -> &'static str {
    match provider {
        LinkProvider::Alice => "Алису",
        LinkProvider::Salut => "Салют",
    }
}

async fn authorize_page(
    State(st): State<AppState>,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    let voice = &st.config.voice;
    let Some(client) = known_target(voice, &params.client_id, &params.redirect_uri) else {
        return refused(
            StatusCode::BAD_REQUEST,
            "Ссылка не подходит. Начните связывание заново.",
        );
    };
    if params.response_type != "code" {
        return refused(
            StatusCode::BAD_REQUEST,
            "Ссылка не подходит. Начните связывание заново.",
        );
    }
    html(
        StatusCode::OK,
        page::form(
            &PageFields {
                client_id: &client.id,
                redirect_uri: &params.redirect_uri,
                state: &params.state,
                scope: &params.scope,
                assistant: &voice.skill_name,
                target: target_name(client.provider),
            },
            None,
        ),
    )
}

async fn authorize_submit(
    State(st): State<AppState>,
    parts: Parts,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let voice = &st.config.voice;
    let Some(client) = known_target(voice, &form.client_id, &form.redirect_uri) else {
        return refused(
            StatusCode::BAD_REQUEST,
            "Ссылка не подходит. Начните связывание заново.",
        );
    };
    let fields = PageFields {
        client_id: &client.id,
        redirect_uri: &form.redirect_uri,
        state: &form.state,
        scope: &form.scope,
        assistant: &voice.skill_name,
        target: target_name(client.provider),
    };
    let identity = match parts.extensions.get::<ConnectInfo<SocketAddr>>() {
        Some(ConnectInfo(address)) => format!(
            "oauth:{}",
            st.admission.client_key(*address, &parts.headers)
        ),
        None => "oauth:unknown".to_owned(),
    };
    match st
        .admission
        .check_key(Endpoint::VoiceCodeTry, &identity)
        .await
    {
        Ok(()) => {}
        Err(AdmissionRejection::Limited { .. }) => {
            return html(
                StatusCode::TOO_MANY_REQUESTS,
                page::form(&fields, Some("Слишком много попыток. Подождите минуту.")),
            );
        }
        Err(AdmissionRejection::Unavailable) => {
            return html(
                StatusCode::SERVICE_UNAVAILABLE,
                page::form(&fields, Some("Сервис занят. Попробуйте ещё раз.")),
            );
        }
    }
    let Some(code) = normalize_code(&form.code) else {
        return html(
            StatusCode::BAD_REQUEST,
            page::form(&fields, Some("Нужны шесть цифр.")),
        );
    };
    let user = match st.links.take_code(client.provider, &code).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return html(
                StatusCode::BAD_REQUEST,
                page::form(
                    &fields,
                    Some("Код не подошёл или устарел. Возьмите новый в приложении."),
                ),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "link code lookup failed");
            return html(
                StatusCode::SERVICE_UNAVAILABLE,
                page::form(&fields, Some("Сервис занят. Попробуйте ещё раз.")),
            );
        }
    };
    let issued = st
        .links
        .issue_auth_code(&user, client.provider, &client.id, &form.redirect_uri)
        .await;
    let Ok(auth_code) = issued else {
        return html(
            StatusCode::SERVICE_UNAVAILABLE,
            page::form(&fields, Some("Сервис занят. Попробуйте ещё раз.")),
        );
    };
    let Some(location) = redirect_with_code(&form.redirect_uri, &auth_code, &form) else {
        return refused(StatusCode::BAD_REQUEST, "Ссылка не подходит.");
    };
    let Ok(location) = HeaderValue::from_str(&location) else {
        return refused(StatusCode::BAD_REQUEST, "Ссылка не подходит.");
    };
    (
        StatusCode::FOUND,
        [
            (LOCATION, location),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response()
}

fn oauth_error(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        [
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (PRAGMA, HeaderValue::from_static("no-cache")),
        ],
        Json(json!({ "error": code })),
    )
        .into_response()
}

async fn token(
    State(st): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    let basic = basic_credentials(headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()));
    let (client_id, secret) = match basic {
        Some(pair) => pair,
        None => (form.client_id.clone(), form.client_secret.clone()),
    };
    let Some(client) = st.config.voice.client(&client_id) else {
        return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
    };
    if !secret_matches(&secret, &client.secret_sha256) {
        return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
    }
    let issued = match form.grant_type.as_str() {
        "authorization_code" => {
            if !client.redirect_uris.contains(&form.redirect_uri) {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            st.links
                .redeem_auth_code(&form.code, &client.id, &form.redirect_uri)
                .await
        }
        "refresh_token" => {
            st.links
                .rotate_refresh(&form.refresh_token, &client.id)
                .await
        }
        _ => return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    };
    match issued {
        Ok(Some(tokens)) => (
            StatusCode::OK,
            [
                (CACHE_CONTROL, HeaderValue::from_static("no-store")),
                (PRAGMA, HeaderValue::from_static("no-cache")),
            ],
            Json(json!({
                "access_token": tokens.access_token,
                "token_type": "Bearer",
                "expires_in": tokens.expires_in,
                "refresh_token": tokens.refresh_token,
            })),
        )
            .into_response(),
        Ok(None) => oauth_error(StatusCode::BAD_REQUEST, "invalid_grant"),
        Err(error) => {
            tracing::warn!(%error, "token exchange failed");
            oauth_error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
    }
}
