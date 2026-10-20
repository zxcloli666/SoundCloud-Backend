use std::fs;
use std::path::Path;

use anyhow::{Context, ensure};
use redact::Secret;

use crate::modules::chat::config::ModerationConfig;
use crate::modules::chat::moderation::verdict::Verdict;
use crate::modules::chat::{ChatError, ChatResult};

pub const KEY_FILE: &str = "mod-ai-key";

pub struct AiClient {
    endpoint: Option<Endpoint>,
}

struct Endpoint {
    url: String,
    model: String,
    key: Secret<String>,
}

pub struct Prompt {
    pub system: String,
    pub user: String,
}

impl AiClient {
    pub fn load(config: &ModerationConfig, keys_dir: &Path) -> anyhow::Result<Self> {
        if !config.enabled {
            return Ok(Self { endpoint: None });
        }
        let (Some(url), Some(model)) = (config.url.clone(), config.model.clone()) else {
            anyhow::bail!("CHAT_MOD_AI_ENABLED needs CHAT_MOD_AI_URL and CHAT_MOD_AI_MODEL");
        };
        let key = fs::read_to_string(keys_dir.join(KEY_FILE))
            .with_context(|| format!("{KEY_FILE} is not readable"))?;
        let key = key.trim().to_owned();
        ensure!(!key.is_empty(), "{KEY_FILE} is empty");
        Ok(Self {
            endpoint: Some(Endpoint {
                url,
                model,
                key: Secret::new(key),
            }),
        })
    }

    pub fn enabled(&self) -> bool {
        self.endpoint.is_some()
    }

    pub async fn verdict(&self, _prompt: &Prompt) -> ChatResult<Option<Verdict>> {
        Err(ChatError::Unimplemented)
    }
}
