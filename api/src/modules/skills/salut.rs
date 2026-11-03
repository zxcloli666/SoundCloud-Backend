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
    let voice = &st.config.voice;
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
        Some(grant) => linked(&st, &grant, &request, &phrases).await,
        None => unlinked(&st, &sub, &request, &phrases).await,
    };
    Json(answer(&request, &said, false)).into_response()
}

async fn linked(
    st: &AppState,
    grant: &Grant,
    request: &SalutRequest,
    phrases: &Phrases<'_>,
) -> Said {
    let utterance = request.utterance();
    if request.message_name == "RUN_APP" || utterance.is_empty() {
        return Said::plain(&phrases.hello());
    }
    if wants_help(utterance) {
        return Said::plain(&phrases.help());
    }
    converse::run(st, grant, utterance, st.config.voice.salut_wait, phrases).await
}

async fn unlinked(st: &AppState, sub: &str, request: &SalutRequest, phrases: &Phrases<'_>) -> Said {
    let spoken = request.spoken_code();
    if spoken.is_empty() {
        return Said::plain(phrases.say_code());
    }
    let Some(code) = normalize_code(&spoken) else {
        return Said::plain(phrases.code_needs_six());
    };
    match st
        .admission
        .check_key(Endpoint::VoiceCodeTry, &format!("salut:{sub}"))
        .await
    {
        Ok(()) => {}
        Err(AdmissionRejection::Limited { .. }) => return Said::plain(phrases.code_throttled()),
        Err(AdmissionRejection::Unavailable) => return Said::plain(phrases.busy()),
    }
    let user = match st.links.take_code(LinkProvider::Salut, &code).await {
        Ok(Some(user)) => user,
        Ok(None) => return Said::plain(phrases.code_wrong()),
        Err(error) => {
            tracing::warn!(%error, "salut code lookup failed");
            return Said::plain(phrases.busy());
        }
    };
    match st.links.bind_salut(&user, sub).await {
        Ok(()) => Said::plain(phrases.linked()),
        Err(error) => {
            tracing::warn!(%error, "salut binding failed");
            Said::plain(phrases.busy())
        }
    }
}
