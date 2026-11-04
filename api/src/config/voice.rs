use std::time::Duration;

use backend_contracts::remote::LinkProvider;
use redact::Secret;
use serde::Deserialize;

use super::{env_opt, env_str, env_u64};

#[derive(Clone, Debug)]
pub struct OAuthClient {
    pub id: String,
    pub provider: LinkProvider,
    pub secret_sha256: Vec<u8>,
    pub redirect_uris: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct VoiceCfg {
    pub assistant_name: String,
    pub skill_name: String,
    pub alice_skill_id: String,
    pub salut_token: Secret<String>,
    pub alice_wait: Duration,
    pub salut_wait: Duration,
    pub oauth_clients: Vec<OAuthClient>,
    pub enabled: bool,
    pub confirm_per_user_day: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientEntry {
    id: String,
    provider: LinkProvider,
    secret_sha256: String,
    redirect_uris: Vec<String>,
}

impl VoiceCfg {
    pub fn from_env() -> Self {
        let assistant_name = env_str("VOICE_ASSISTANT_NAME", "Лоля");
        let skill_name = env_opt("VOICE_SKILL_NAME").unwrap_or_else(|| assistant_name.clone());
        let (oauth_clients, enabled) = match parse_clients(&env_str("VOICE_OAUTH_CLIENTS", "[]")) {
            Ok(clients) => (clients, true),
            Err(error) => {
                tracing::error!(%error, "voice is disabled until VOICE_OAUTH_CLIENTS is fixed");
                (Vec::new(), false)
            }
        };
        Self {
            assistant_name,
            skill_name,
            alice_skill_id: env_str("VOICE_ALICE_SKILL_ID", ""),
            salut_token: env_str("VOICE_SALUT_TOKEN", "").into(),
            alice_wait: Duration::from_millis(env_u64("VOICE_ALICE_WAIT_MS", 4000)),
            salut_wait: Duration::from_millis(env_u64("VOICE_SALUT_WAIT_MS", 5500)),
            oauth_clients,
            enabled,
            confirm_per_user_day: env_u64("VOICE_CONFIRM_PER_USER_DAY", 20),
        }
    }

    pub fn client(&self, id: &str) -> Option<&OAuthClient> {
        self.oauth_clients.iter().find(|client| client.id == id)
    }
}

pub fn parse_clients(raw: &str) -> Result<Vec<OAuthClient>, String> {
    let entries: Vec<ClientEntry> = serde_json::from_str(raw)
        .map_err(|error| format!("VOICE_OAUTH_CLIENTS is not a valid client list: {error}"))?;
    entries
        .into_iter()
        .map(|entry| {
            let secret_sha256 = hex::decode(entry.secret_sha256.trim()).unwrap_or_default();
            if entry.id.is_empty() || secret_sha256.len() != 32 || entry.redirect_uris.is_empty() {
                return Err(format!(
                    "VOICE_OAUTH_CLIENTS entry {} needs an id, a sha256 hex secret and redirect uris",
                    entry.id
                ));
            }
            Ok(OAuthClient {
                id: entry.id,
                provider: entry.provider,
                secret_sha256,
                redirect_uris: entry.redirect_uris,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_come_from_json_with_a_hashed_secret() {
        let hash = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(b"s3cret"));
        let raw = format!(
            r#"[{{"id":"yandex","provider":"alice","secretSha256":"{hash}","redirectUris":["https://social.yandex.net/broker/redirect"]}}]"#
        );
        let clients = parse_clients(&raw).unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].provider, LinkProvider::Alice);
        assert_eq!(clients[0].secret_sha256.len(), 32);
    }

    #[test]
    fn an_empty_list_means_no_clients() {
        assert!(parse_clients("[]").unwrap().is_empty());
    }

    #[test]
    fn a_plain_text_secret_is_refused() {
        let error = parse_clients(
            r#"[{"id":"x","provider":"alice","secretSha256":"plain","redirectUris":["https://a"]}]"#,
        )
        .unwrap_err();
        assert!(error.contains("needs an id"));
    }

    #[test]
    fn broken_json_is_an_error_not_a_panic() {
        assert!(parse_clients("{not json").is_err());
        assert!(parse_clients("").is_err());
    }
}
