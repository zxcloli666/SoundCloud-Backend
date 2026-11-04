use axum::Json;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::converse::{self, Said};
use super::phrases::{Phrases, wants_help};
use super::speech::spoken_digits;
use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::error::AppError;
use crate::modules::remote::confirm::{SalutTicket, Status};
use crate::modules::remote::linking::{self, Found, Pairing};
use crate::modules::remote::links::Grant;
use crate::modules::remote::secrets::{normalize_code, token_matches};
use crate::state::AppState;
use backend_contracts::remote::LinkProvider;

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

pub(super) fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let custom = headers
        .get("x-voice-token")
        .and_then(|value| value.to_str().ok());
    [bearer, custom]
        .into_iter()
        .flatten()
        .any(|provided| token_matches(provided.trim(), expected))
}

pub async fn webhook(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SalutRequest>,
) -> Response {
    let started = std::time::Instant::now();
    let voice = &st.config.voice;
    if !voice.enabled {
        return AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_disabled",
            "Voice is not configured",
        )
        .into_response();
    }
    if !authorized(&headers, voice.salut_token.expose()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if request.message_name == "CLOSE_APP" {
        return Json(json!({})).into_response();
    }
    let phrases = Phrases {
        name: &voice.assistant_name,
    };
    let Some(sub) = request.sub().map(str::to_owned) else {
        return Json(answer(&request, &Said::plain(phrases.repeat()), true)).into_response();
    };
    let grant = match st.links.find_salut(&sub).await {
        Ok(grant) => grant,
        Err(error) => {
            tracing::warn!(%error, "salut binding lookup failed");
            return Json(answer(&request, &Said::plain(phrases.busy()), true)).into_response();
        }
    };
    let said = match grant {
        Some(grant) => linked(&st, &grant, &request, started, &phrases).await,
        None => unlinked(&st, &sub, &request, &phrases).await,
    };
    Json(answer(&request, &said, false)).into_response()
}

async fn linked(
    st: &AppState,
    grant: &Grant,
    request: &SalutRequest,
    started: std::time::Instant,
    phrases: &Phrases<'_>,
) -> Said {
    let utterance = request.utterance();
    if request.message_name == "RUN_APP" || utterance.is_empty() {
        return Said::plain(&phrases.hello());
    }
    if wants_help(utterance) {
        return Said::plain(&phrases.help());
    }
    converse::run(
        st,
        grant,
        utterance,
        started,
        st.config.voice.salut_wait,
        phrases,
    )
    .await
}

const TICKET_PENDING_SECS: u64 = 90;
const TICKET_DONE_SECS: u64 = 600;

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
    let Some(code) = normalize_code(&spoken) else {
        return Said::plain(phrases.code_needs_six());
    };
    match st
        .admission
        .check_key(Endpoint::VoiceSalutSub, sub)
        .await
    {
        Ok(()) => {}
        Err(AdmissionRejection::Limited { .. }) => return Said::plain(phrases.code_throttled()),
        Err(AdmissionRejection::Unavailable) => return Said::plain(phrases.busy()),
    }
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
    let pending = SalutTicket {
        status: Status::Pending,
        user: pairing.user.clone(),
    };
    if let Err(error) = st
        .confirm
        .put(&ticket_key(sub), &pending, TICKET_PENDING_SECS)
        .await
    {
        tracing::warn!(%error, "salut link state was not stored");
        st.confirm.release(pairing.code_id).await;
        return Said::plain(phrases.busy());
    }
    tokio::spawn(settle(st.clone(), pairing, sub.to_owned()));
    Said::plain(phrases.confirm_on_pc())
}

async fn settle(st: AppState, pairing: Pairing, sub: String) {
    let status = linking::ask_desktop(&st, &pairing).await;
    let done = SalutTicket {
        status,
        user: pairing.user,
    };
    if let Err(error) = st
        .confirm
        .put(&ticket_key(&sub), &done, TICKET_DONE_SECS)
        .await
    {
        tracing::warn!(%error, "salut link result was not stored");
    }
}

async fn after_confirmation(
    st: &AppState,
    sub: &str,
    ticket: SalutTicket,
    phrases: &Phrases<'_>,
) -> Said {
    if ticket.status == Status::Pending {
        return Said::plain(phrases.confirm_on_pc());
    }
    let _ = st.confirm.take::<SalutTicket>(&ticket_key(sub)).await;
    match ticket.status {
        Status::Approved => match st.links.bind_salut(&ticket.user, sub).await {
            Ok(()) => Said::plain(phrases.linked()),
            Err(error) => {
                tracing::warn!(%error, "salut binding failed");
                let _ = st
                    .confirm
                    .put(&ticket_key(sub), &ticket, TICKET_DONE_SECS)
                    .await;
                Said::plain(phrases.busy())
            }
        },
        Status::Denied => Said::plain(phrases.link_denied()),
        Status::Offline => Said::plain(phrases.link_offline()),
        Status::Silent => Said::plain(phrases.link_silent()),
        _ => Said::plain(phrases.busy()),
    }
}
