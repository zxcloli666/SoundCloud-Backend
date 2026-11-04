use std::time::Instant;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use backend_contracts::remote::LinkProvider;
use serde::Deserialize;
use serde_json::{Value, json};

use super::converse::{self, Said, caller};
use super::phrases::{Phrases, wants_help};
use super::speech::spoken_digits;
use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::modules::remote::confirm::{SalutTicket, Status};
use crate::modules::remote::handlers::voice_disabled;
use crate::modules::remote::linking::{self, Found, REVIEW_USER, is_review_code};
use crate::modules::remote::secrets::{normalize_code, token_matches};
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SalutRequest {
    #[serde(default)]
    pub message_name: String,
    #[serde(default)]
    pub session_id: Value,
    #[serde(default)]
    pub message_id: Value,
    #[serde(default)]
    pub uuid: Value,
    #[serde(default)]
    pub payload: SalutPayload,
}

#[derive(Debug, Default, Deserialize)]
pub struct SalutPayload {
    #[serde(default)]
    pub message: Option<SalutMessage>,
    #[serde(default)]
    pub app_info: Option<SalutAppInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SalutAppInfo {
    #[serde(default)]
    pub project_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct SalutMessage {
    #[serde(default)]
    pub original_text: String,
    #[serde(default)]
    pub asr_normalized_message: String,
}

impl SalutRequest {
    pub fn sub(&self) -> Option<&str> {
        self.uuid
            .get("sub")
            .and_then(Value::as_str)
            .filter(|sub| !sub.is_empty())
    }

    pub fn project_id(&self) -> &str {
        self.payload
            .app_info
            .as_ref()
            .map_or("", |info| info.project_id.as_str())
    }

    pub fn utterance(&self) -> &str {
        self.payload
            .message
            .as_ref()
            .map_or("", |message| message.original_text.trim())
    }

    pub fn spoken_code(&self) -> String {
        let Some(message) = &self.payload.message else {
            return String::new();
        };
        let from_asr = spoken_digits(&message.asr_normalized_message);
        if !from_asr.is_empty() {
            return from_asr;
        }
        spoken_digits(&message.original_text)
    }
}

pub fn answer(request: &SalutRequest, said: &Said, finished: bool) -> Value {
    json!({
        "messageName": "ANSWER_TO_USER",
        "sessionId": request.session_id,
        "messageId": request.message_id,
        "uuid": request.uuid,
        "payload": {
            "pronounceText": said.speech,
            "pronounceTextType": "application/text",
            "items": [{ "bubble": { "text": said.text } }],
            "auto_listening": false,
            "finished": finished,
        },
    })
}

pub(super) fn authorized(headers: &HeaderMap, path_token: Option<&str>, expected: &str) -> bool {
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let custom = headers
        .get("x-voice-token")
        .and_then(|value| value.to_str().ok());
    [bearer, custom, path_token]
        .into_iter()
        .flatten()
        .any(|provided| token_matches(provided.trim(), expected))
}

pub(super) fn admitted(
    token: &str,
    project_id: &str,
    headers: &HeaderMap,
    path_token: Option<&str>,
    request: &SalutRequest,
) -> Result<(), StatusCode> {
    if token.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if !authorized(headers, path_token, token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !project_id.is_empty() && !token_matches(request.project_id(), project_id) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

pub async fn webhook(
    State(st): State<AppState>,
    parts: Parts,
    Json(request): Json<SalutRequest>,
) -> Response {
    serve(st, parts, None, request).await
}

pub async fn webhook_with_token(
    State(st): State<AppState>,
    Path(token): Path<String>,
    parts: Parts,
    Json(request): Json<SalutRequest>,
) -> Response {
    serve(st, parts, Some(token), request).await
}

async fn serve(
    st: AppState,
    parts: Parts,
    path_token: Option<String>,
    request: SalutRequest,
) -> Response {
    let started = Instant::now();
    let voice = &st.config.voice;
    if !voice.enabled || !voice.salut_enabled() {
        return voice_disabled();
    }
    if let Err(status) = admitted(
        voice.salut_token.expose(),
        &voice.salut_project_id,
        &parts.headers,
        path_token.as_deref(),
        &request,
    ) {
        return status.into_response();
    }
    if request.message_name == "CLOSE_APP" {
        return Json(json!({})).into_response();
    }
    let phrases = Phrases {
        name: &voice.assistant_name,
    };
    if let Some(said) = greeting(&request, &phrases) {
        return Json(answer(&request, &said, false)).into_response();
    }
    let Some(sub) = request.sub() else {
        return Json(answer(&request, &Said::plain(phrases.repeat()), true)).into_response();
    };
    let budget = voice.salut_wait.saturating_sub(started.elapsed());
    let said = match tokio::time::timeout(
        budget,
        respond(&st, &parts, sub, &request, started, &phrases),
    )
    .await
    {
        Ok(said) => said,
        Err(_) => Said::plain(phrases.busy()),
    };
    Json(answer(&request, &said, false)).into_response()
}

fn greeting(request: &SalutRequest, phrases: &Phrases<'_>) -> Option<Said> {
    let utterance = request.utterance();
    if request.message_name == "RUN_APP" || utterance.is_empty() {
        return Some(Said::plain(&phrases.hello()));
    }
    wants_help(utterance).then(|| Said::plain(&phrases.help()))
}

async fn respond(
    st: &AppState,
    parts: &Parts,
    sub: &str,
    request: &SalutRequest,
    started: Instant,
    phrases: &Phrases<'_>,
) -> Said {
    let caller = format!("salut:{}", caller(st, parts));
    match st
        .admission
        .check_key(Endpoint::VoiceSalutIp, &caller)
        .await
    {
        Ok(()) => {}
        Err(AdmissionRejection::Limited { .. }) => return Said::plain(phrases.too_fast()),
        Err(AdmissionRejection::Unavailable) => return Said::plain(phrases.busy()),
    }
    let grant = match st.links.find_salut(sub).await {
        Ok(grant) => grant,
        Err(error) => {
            tracing::warn!(%error, "salut binding lookup failed");
            return Said::plain(phrases.busy());
        }
    };
    match grant {
        Some(grant) => {
            converse::run(
                st,
                &grant,
                request.utterance(),
                started,
                st.config.voice.salut_wait,
                phrases,
            )
            .await
        }
        None => unlinked(st, sub, request, phrases).await,
    }
}

const TICKET_SECS: u64 = 600;

fn ticket_key(sub: &str) -> String {
    format!("salut:{sub}")
}

async fn unlinked(st: &AppState, sub: &str, request: &SalutRequest, phrases: &Phrases<'_>) -> Said {
    match st.confirm.get::<SalutTicket>(&ticket_key(sub)).await {
        Ok(Some(ticket)) => return after_confirmation(st, sub, ticket, phrases).await,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "salut link state lookup failed");
            return Said::plain(phrases.busy());
        }
    }
    let spoken = request.spoken_code();
    if spoken.is_empty() {
        return Said::plain(phrases.say_code());
    }
    match st.admission.check_key(Endpoint::VoiceSalutSub, sub).await {
        Ok(()) => {}
        Err(AdmissionRejection::Limited { .. }) => return Said::plain(phrases.code_throttled()),
        Err(AdmissionRejection::Unavailable) => return Said::plain(phrases.busy()),
    }
    if is_review_code(&st.config.voice, &spoken) {
        return bind(st, REVIEW_USER, sub, phrases).await;
    }
    let Some(code) = normalize_code(&spoken) else {
        return Said::plain(phrases.code_needs_six());
    };
    let pairing = match linking::find(st, LinkProvider::Salut, &code).await {
        Ok(Found::Ready(pairing)) => pairing,
        Ok(Found::NoCode) => return Said::plain(phrases.code_wrong()),
        Ok(Found::InUse) => return Said::plain(phrases.confirm_on_pc()),
        Ok(Found::Capped) => return Said::plain(phrases.code_capped()),
        Err(error) => {
            tracing::warn!(%error, "salut code lookup failed");
            return Said::plain(phrases.busy());
        }
    };
    let mut ticket = SalutTicket {
        status: Status::Pending,
        user: pairing.user.clone(),
        hint: pairing.hint.clone(),
        confirming: None,
    };
    match linking::ask(st, &pairing).await {
        Ok(confirming) => ticket.confirming = Some(confirming),
        Err(status) => ticket.status = status,
    }
    if let Err(error) = st.confirm.put(&ticket_key(sub), &ticket, TICKET_SECS).await {
        tracing::warn!(%error, "salut link state was not stored");
        st.confirm.release(pairing.code_id, &pairing.holder).await;
        return Said::plain(phrases.busy());
    }
    Said::plain(&phrases.confirm_with_hint(&ticket.hint))
}

async fn bind(st: &AppState, user: &str, sub: &str, phrases: &Phrases<'_>) -> Said {
    match st.links.bind_salut(user, sub).await {
        Ok(()) => Said::plain(phrases.linked()),
        Err(error) => {
            tracing::warn!(%error, "salut binding failed");
            Said::plain(phrases.busy())
        }
    }
}

async fn after_confirmation(
    st: &AppState,
    sub: &str,
    mut ticket: SalutTicket,
    phrases: &Phrases<'_>,
) -> Said {
    if ticket.status == Status::Pending {
        let settled = match &ticket.confirming {
            Some(confirming) => linking::outcome(st, confirming).await,
            None => Some(Status::Failed),
        };
        match settled {
            Some(status) => ticket.status = status,
            None => return Said::plain(&phrases.confirm_with_hint(&ticket.hint)),
        }
    }
    let _ = st.confirm.take::<SalutTicket>(&ticket_key(sub)).await;
    match ticket.status {
        Status::Approved => match st.links.bind_salut(&ticket.user, sub).await {
            Ok(()) => Said::plain(phrases.linked()),
            Err(error) => {
                tracing::warn!(%error, "salut binding failed");
                let _ = st.confirm.put(&ticket_key(sub), &ticket, TICKET_SECS).await;
                Said::plain(phrases.busy())
            }
        },
        Status::Denied => Said::plain(phrases.link_denied()),
        Status::Offline => Said::plain(phrases.link_offline()),
        Status::Silent => Said::plain(phrases.link_silent()),
        _ => Said::plain(phrases.busy()),
    }
}
