use std::collections::BTreeMap;
use std::path::Path;

use llm_ladder::{ConfigError, SecretRef, Settings};
use redact::Secret;
use serde::Deserialize;

use super::env_opt;

pub const DEFAULT_PERSONA: &str = "default";

const MAX_PERSONA_ID_LEN: usize = 64;
const SPEAK_IN_FLIGHT: usize = 4;

#[derive(Clone, Debug, Default)]
pub struct LlmCfg {
    pub ladders: Settings,
    pub voice_chat: Option<VoiceChatCfg>,
    pub voice_speak: Option<VoiceSpeakCfg>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceChatCfg {
    pub ladder: String,
    pub requests_per_minute: u32,
    pub max_text_chars: usize,
    pub max_history: usize,
    pub max_notes: usize,
    pub max_tools: usize,
    pub max_tool_rounds: usize,
    pub max_system_bytes: usize,
    #[serde(default)]
    pub instructions: String,
    pub personas: BTreeMap<String, PersonaCfg>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaCfg {
    pub prompt: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSpeakCfg {
    pub requests_per_minute: u32,
    #[serde(default = "speak_in_flight")]
    pub max_in_flight: usize,
    pub phrases: Option<PhrasesCfg>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhrasesCfg {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: SecretRef,
    pub secret_key: SecretRef,
}

pub struct PhraseKeys {
    pub access_key: Secret<String>,
    pub secret_key: Secret<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostSections {
    voice_chat: Option<VoiceChatCfg>,
    voice_speak: Option<VoiceSpeakCfg>,
}

impl LlmCfg {
    pub fn from_env() -> Self {
        let Some(path) = env_opt("LLM_CONFIG") else {
            return Self::default();
        };
        let loaded = std::fs::read_to_string(Path::new(&path))
            .map_err(|error| error.to_string())
            .and_then(|text| Self::parse(&text).map_err(|error| error.to_string()));
        loaded.unwrap_or_else(|reason| panic!("LLM_CONFIG {path} is refused: {reason}"))
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let (ladders, host): (Settings, HostSections) = Settings::parse(text)?;
        if let Some(chat) = &host.voice_chat {
            chat.validate(&ladders)
                .map_err(|reason| ConfigError::Invalid(format!("[voice_chat] {reason}")))?;
        }
        if let Some(speak) = &host.voice_speak {
            speak
                .validate()
                .map_err(|reason| ConfigError::Invalid(format!("[voice_speak] {reason}")))?;
        }
        Ok(Self {
            ladders,
            voice_chat: host.voice_chat,
            voice_speak: host.voice_speak,
        })
    }
}

impl VoiceChatCfg {
    pub fn persona(&self, id: &str) -> &str {
        let persona = self
            .personas
            .get(id)
            .or_else(|| self.personas.get(DEFAULT_PERSONA));
        persona.map_or("", |persona| persona.prompt.trim())
    }

    fn validate(&self, ladders: &Settings) -> Result<(), String> {
        if !ladders.ladders.contains_key(&self.ladder) {
            return Err(format!("names an unknown ladder {}", self.ladder));
        }
        if !self.personas.contains_key(DEFAULT_PERSONA) {
            return Err(format!("needs [voice_chat.personas.{DEFAULT_PERSONA}]"));
        }
        if let Some(id) = self.personas.keys().find(|id| !persona_id(id)) {
            return Err(format!("persona id {id} is not a voice id"));
        }
        if let Some(id) = self.blank_persona() {
            return Err(format!("persona {id} has an empty prompt"));
        }
        let limits = [
            ("requests_per_minute", u64::from(self.requests_per_minute)),
            ("max_text_chars", self.max_text_chars as u64),
            ("max_tools", self.max_tools as u64),
            ("max_tool_rounds", self.max_tool_rounds as u64),
            ("max_system_bytes", self.max_system_bytes as u64),
        ];
        if let Some((name, _)) = limits.iter().find(|(_, value)| *value == 0) {
            return Err(format!("{name} must be above zero"));
        }
        match ladders.max_system_bytes(&self.ladder) {
            Some(most) if self.max_system_bytes > most => Err(format!(
                "max_system_bytes is above {most}, the most a worker step of ladder {} takes",
                self.ladder
            )),
            _ => Ok(()),
        }
    }

    fn blank_persona(&self) -> Option<&str> {
        self.personas
            .iter()
            .find(|(_, persona)| persona.prompt.trim().is_empty())
            .map(|(id, _)| id.as_str())
    }
}

impl VoiceSpeakCfg {
    fn validate(&self) -> Result<(), String> {
        if self.requests_per_minute == 0 {
            return Err("requests_per_minute must be above zero".to_owned());
        }
        if self.max_in_flight == 0 {
            return Err("max_in_flight must be above zero".to_owned());
        }
        self.phrases.as_ref().map_or(Ok(()), PhrasesCfg::validate)
    }
}

impl PhrasesCfg {
    pub fn keys(&self) -> Result<PhraseKeys, String> {
        Ok(PhraseKeys {
            access_key: resolved("access_key", &self.access_key)?,
            secret_key: resolved("secret_key", &self.secret_key)?,
        })
    }

    fn validate(&self) -> Result<(), String> {
        let endpoint = self.endpoint.trim();
        if endpoint.is_empty() {
            return Err("phrases.endpoint is empty".to_owned());
        }
        if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
            return Err("phrases.endpoint is not an http or https URL".to_owned());
        }
        if self.bucket.trim().is_empty() {
            return Err("phrases.bucket is empty".to_owned());
        }
        self.keys().map(|_| ())
    }
}

fn resolved(field: &str, secret: &SecretRef) -> Result<Secret<String>, String> {
    secret
        .resolve()
        .ok_or_else(|| format!("phrases.{field} names a variable that is unset or empty"))
}

fn speak_in_flight() -> usize {
    SPEAK_IN_FLIGHT
}

fn persona_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_PERSONA_ID_LEN
        && id
            .chars()
            .all(|symbol| symbol.is_ascii_alphanumeric() || matches!(symbol, '_' | '-'))
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
