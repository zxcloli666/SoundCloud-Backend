use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::response::{IntoResponse, Response};
use backend_contracts::remote::LinkProvider;
use serde::Deserialize;
use serde_json::{Value, json};

use super::converse::{self, Said};
use super::phrases::{Phrases, wants_help};
use crate::error::AppError;
use crate::modules::remote::links::Grant;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub struct AliceRequest {
    #[serde(default)]
    pub session: AliceSession,
    #[serde(default)]
    pub request: AliceInner,
}

#[derive(Debug, Default, Deserialize)]
pub struct AliceSession {
    #[serde(default)]
    pub skill_id: String,
    #[serde(default)]
    pub new: bool,
    #[serde(default)]
    pub user: Option<AliceUser>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AliceUser {
    #[serde(default)]
    pub access_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AliceInner {
    #[serde(default)]
    pub original_utterance: String,
    #[serde(default)]
    pub command: String,
}

impl AliceRequest {
    pub fn utterance(&self) -> &str {
        let original = self.request.original_utterance.trim();
        if original.is_empty() {
            self.request.command.trim()
        } else {
            original
        }
    }

    pub fn token(&self, headers: &HeaderMap) -> Option<String> {
        let bearer = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|token| !token.is_empty());
        bearer.map(str::to_owned).or_else(|| {
            self.session
                .user
                .as_ref()
                .and_then(|user| user.access_token.clone())
                .filter(|token| !token.is_empty())
        })
    }
}

pub fn spoken(said: &Said, end_session: bool) -> Value {
    json!({
        "response": { "text": said.text, "tts": said.speech, "end_session": end_session },
        "version": "1.0",
    })
}

pub fn start_linking(text: &str) -> Value {
    json!({
        "response": {
            "text": text,
            "end_session": false,
            "directives": { "start_account_linking": {} },
        },
        "version": "1.0",
    })
}

pub fn pong() -> Value {
    json!({ "response": { "text": "pong", "end_session": true }, "version": "1.0" })
}

pub async fn webhook(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AliceRequest>,
) -> Response {
    if request.request.original_utterance.trim() == "ping" {
        return Json(pong()).into_response();
    }
    let voice = &st.config.voice;
    if voice.alice_skill_id.is_empty() || request.session.skill_id != voice.alice_skill_id {
        return AppError::forbidden("Unknown skill").into_response();
    }
    let phrases = Phrases {
        name: &voice.assistant_name,
    };
    let utterance = request.utterance().to_owned();
    let grant = match request.token(&headers) {
        Some(token) => match st.links.authenticate(&token).await {
            Ok(grant) => grant.filter(|grant| grant.provider == LinkProvider::Alice),
            Err(error) => {
                tracing::warn!(%error, "alice token lookup failed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(spoken(&Said::plain(phrases.busy()), true)),
                )
                    .into_response();
            }
        },
        None => None,
    };
    let Some(grant) = grant else {
        return Json(start_linking(phrases.link_needed())).into_response();
    };
    Json(answer(&st, &grant, &request, &utterance, &phrases).await).into_response()
}

async fn answer(
    st: &AppState,
    grant: &Grant,
    request: &AliceRequest,
    utterance: &str,
    phrases: &Phrases<'_>,
) -> Value {
    if utterance.is_empty() {
        return spoken(&Said::plain(&phrases.hello()), false);
    }
    if wants_help(utterance) {
        return spoken(&Said::plain(&phrases.help()), false);
    }
    let said = converse::run(st, grant, utterance, st.config.voice.alice_wait, phrases).await;
    spoken(&said, request.session.new)
}
