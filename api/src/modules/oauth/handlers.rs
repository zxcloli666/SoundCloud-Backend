use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION, PRAGMA,
    X_FRAME_OPTIONS,
};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use backend_contracts::remote::LinkProvider;
use serde_json::json;

use super::page::{self, PageFields};
use super::request::{
    AuthorizeForm, AuthorizeParams, StatusParams, TokenForm, basic_credentials, known_target,
    redirect_with_code,
};
use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::modules::remote::confirm::{PageTicket, Status};
use crate::modules::remote::handlers::voice_disabled;
use crate::modules::remote::linking::{self, Found, Pairing, REVIEW_USER, is_review_code};
use crate::modules::remote::secrets::{normalize_code, random_token, secret_matches};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/oauth/authorize",
            get(authorize_page).post(authorize_submit),
        )
        .route("/oauth/authorize/status", get(authorize_status))
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
            (X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
            (
                CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("frame-ancestors 'none'"),
            ),
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

const TICKET_TTL_SECS: u64 = 600;
const PAGE_WAIT_SECS: u64 = 60;
const PENDING_TTL_SECS: u64 = 90;

async fn authorize_page(
    State(st): State<AppState>,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    let voice = &st.config.voice;
    if !voice.enabled {
        return voice_disabled();
    }
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

fn busy_form(fields: &PageFields<'_>) -> Response {
    html(
        StatusCode::SERVICE_UNAVAILABLE,
        page::form(fields, Some("Сервис занят. Попробуйте ещё раз.")),
    )
}

fn limited_form(fields: &PageFields<'_>) -> Response {
    html(
        StatusCode::TOO_MANY_REQUESTS,
        page::form(fields, Some("Слишком много попыток. Подождите минуту.")),
    )
}

async fn within_limit(
    st: &AppState,
    endpoint: Endpoint,
    identity: &str,
    fields: &PageFields<'_>,
) -> Option<Response> {
    match st.admission.check_key(endpoint, identity).await {
        Ok(()) => None,
        Err(AdmissionRejection::Limited { .. }) => Some(limited_form(fields)),
        Err(AdmissionRejection::Unavailable) => Some(busy_form(fields)),
    }
}

async fn authorize_submit(
    State(st): State<AppState>,
    parts: Parts,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let voice = &st.config.voice;
    if !voice.enabled {
        return voice_disabled();
    }
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
    let address = match parts.extensions.get::<ConnectInfo<SocketAddr>>() {
        Some(ConnectInfo(address)) => st.admission.network_key(*address, &parts.headers),
        None => "unknown".to_owned(),
    };
    if let Some(refusal) = within_limit(
        &st,
        Endpoint::VoicePageIp,
        &format!("oauth:{address}"),
        &fields,
    )
    .await
    {
        return refusal;
    }
    if let Some(refusal) = within_limit(&st, Endpoint::VoicePageClient, &client.id, &fields).await {
        return refusal;
    }
    if is_review_code(voice, &form.code) {
        return review_redirect(&st, client.provider, &form, &fields).await;
    }
    let Some(code) = normalize_code(&form.code) else {
        return html(
            StatusCode::BAD_REQUEST,
            page::form(&fields, Some("Нужны шесть цифр.")),
        );
    };
    let pairing = match linking::find(&st, client.provider, &code).await {
        Ok(Found::Ready(pairing)) => pairing,
        Ok(Found::NoCode) => {
            return html(
                StatusCode::BAD_REQUEST,
                page::form(
                    &fields,
                    Some("Код не подошёл или устарел. Возьмите новый в приложении."),
                ),
            );
        }
        Ok(Found::InUse) => {
            return html(
                StatusCode::CONFLICT,
                page::form(
                    &fields,
                    Some("Этот код уже проверяется. Подождите минуту или возьмите новый."),
                ),
            );
        }
        Ok(Found::Capped) => {
            return html(
                StatusCode::TOO_MANY_REQUESTS,
                page::form(
                    &fields,
                    Some("На сегодня слишком много запросов на подтверждение."),
                ),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "link code lookup failed");
            return busy_form(&fields);
        }
    };
    let ticket = random_token();
    let record = PageTicket {
        status: Status::Pending,
        client_id: client.id.clone(),
        redirect_uri: form.redirect_uri.clone(),
        state: form.state.clone(),
        scope: form.scope.clone(),
        location: String::new(),
        hint: pairing.hint.clone(),
    };
    if let Err(error) = st
        .confirm
        .put(&format!("ticket:{ticket}"), &record, PENDING_TTL_SECS)
        .await
    {
        tracing::warn!(%error, "link ticket was not stored");
        st.confirm.release(pairing.code_id).await;
        return busy_form(&fields);
    }
    let hint = pairing.hint.clone();
    tokio::spawn(settle_page(st.clone(), pairing, ticket.clone(), record));
    html(
        StatusCode::OK,
        page::pending(&ticket, &voice.skill_name, &hint),
    )
}

async fn review_redirect(
    st: &AppState,
    provider: LinkProvider,
    form: &AuthorizeForm,
    fields: &PageFields<'_>,
) -> Response {
    let issued = st
        .links
        .issue_auth_code(REVIEW_USER, provider, &form.client_id, &form.redirect_uri)
        .await;
    let code = match issued {
        Ok(code) => code,
        Err(error) => {
            tracing::warn!(%error, "review auth code was not issued");
            return busy_form(fields);
        }
    };
    let Some(location) = redirect_with_code(
        &form.redirect_uri,
        &code,
        &form.state,
        &form.client_id,
        &form.scope,
    )
    .and_then(|location| HeaderValue::from_str(&location).ok()) else {
        return busy_form(fields);
    };
    found(location)
}

fn found(location: HeaderValue) -> Response {
    (
        StatusCode::FOUND,
        [
            (LOCATION, location),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response()
}

async fn settle_page(st: AppState, pairing: Pairing, ticket: String, mut record: PageTicket) {
    let asked = tokio::time::timeout(
        std::time::Duration::from_secs(PAGE_WAIT_SECS + 5),
        linking::ask_desktop(&st, &pairing),
    )
    .await;
    record.status = asked.unwrap_or(Status::Silent);
    if record.status == Status::Approved {
        let issued = st
            .links
            .issue_auth_code(
                &pairing.user,
                pairing.provider,
                &record.client_id,
                &record.redirect_uri,
            )
            .await;
        match issued {
            Ok(code) => {
                match redirect_with_code(
                    &record.redirect_uri,
                    &code,
                    &record.state,
                    &record.client_id,
                    &record.scope,
                ) {
                    Some(location) => record.location = location,
                    None => record.status = Status::Failed,
                }
            }
            Err(error) => {
                tracing::warn!(%error, "auth code was not issued after the confirmation");
                record.status = Status::Failed;
            }
        }
    }
    if let Err(error) = st
        .confirm
        .put(&format!("ticket:{ticket}"), &record, TICKET_TTL_SECS)
        .await
    {
        tracing::warn!(%error, "link ticket result was not stored");
    }
}

fn outcome_text(status: Status) -> &'static str {
    match status {
        Status::Denied => "Связывание отклонено на компьютере.",
        Status::Offline => {
            "Компьютер не отвечает. Откройте SoundCloud Desktop и возьмите новый код."
        }
        Status::Silent => "Компьютер не ответил вовремя. Возьмите новый код в приложении.",
        _ => "Не получилось. Возьмите новый код в приложении.",
    }
}

async fn authorize_status(
    State(st): State<AppState>,
    Query(params): Query<StatusParams>,
) -> Response {
    let voice = &st.config.voice;
    let key = format!("ticket:{}", params.ticket);
    let record = if params.ticket.len() == 43 {
        st.confirm.get::<PageTicket>(&key).await
    } else {
        Ok(None)
    };
    let record = match record {
        Ok(Some(record)) => record,
        Ok(None) => {
            return refused(
                StatusCode::BAD_REQUEST,
                "Время вышло. Начните связывание заново.",
            );
        }
        Err(error) => {
            tracing::warn!(%error, "link ticket lookup failed");
            return refused(
                StatusCode::SERVICE_UNAVAILABLE,
                "Сервис занят. Обновите страницу.",
            );
        }
    };
    if record.status == Status::Pending {
        return html(
            StatusCode::OK,
            page::pending(&params.ticket, &voice.skill_name, &record.hint),
        );
    }
    let _ = st.confirm.take::<PageTicket>(&key).await;
    if record.status == Status::Approved
        && let Ok(location) = HeaderValue::from_str(&record.location)
    {
        return found(location);
    }
    let Some(client) = known_target(voice, &record.client_id, &record.redirect_uri) else {
        return refused(StatusCode::BAD_REQUEST, "Ссылка не подходит.");
    };
    let fields = PageFields {
        client_id: &client.id,
        redirect_uri: &record.redirect_uri,
        state: &record.state,
        scope: &record.scope,
        assistant: &voice.skill_name,
        target: target_name(client.provider),
    };
    html(
        StatusCode::OK,
        page::form(&fields, Some(outcome_text(record.status))),
    )
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
        None => (form.client_id.clone(), form.client_secret.expose().clone()),
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
                .rotate_refresh(form.refresh_token.expose(), &client.id)
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
                "access_token": tokens.access_token.expose(),
                "token_type": "Bearer",
                "expires_in": tokens.expires_in,
                "refresh_token": tokens.refresh_token.expose(),
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
