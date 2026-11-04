use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

use crate::config::{OAuthClient, VoiceCfg};

#[derive(Debug, Deserialize)]
pub struct AuthorizeParams {
    #[serde(default)]
    pub response_type: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeForm {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    #[serde(default)]
    pub grant_type: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub refresh_token: redact::Secret<String>,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: redact::Secret<String>,
}

pub fn known_target<'a>(
    voice: &'a VoiceCfg,
    client_id: &str,
    redirect_uri: &str,
) -> Option<&'a OAuthClient> {
    voice.client(client_id).filter(|client| {
        client
            .redirect_uris
            .iter()
            .any(|known| known == redirect_uri)
    })
}

pub fn basic_credentials(header: Option<&str>) -> Option<(String, String)> {
    let encoded = header?.strip_prefix("Basic ")?;
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    Some((
        urlencoding::decode(id).ok()?.into_owned(),
        urlencoding::decode(secret).ok()?.into_owned(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct StatusParams {
    #[serde(default)]
    pub ticket: String,
}

pub fn redirect_with_code(
    redirect_uri: &str,
    code: &str,
    state: &str,
    client_id: &str,
    scope: &str,
) -> Option<String> {
    let mut url = url::Url::parse(redirect_uri).ok()?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("code", code);
        if !state.is_empty() {
            query.append_pair("state", state);
        }
        query.append_pair("client_id", client_id);
        if !scope.is_empty() {
            query.append_pair("scope", scope);
        }
    }
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_credentials_decode() {
        let header = format!("Basic {}", STANDARD.encode("yandex:s3cret"));
        assert_eq!(
            basic_credentials(Some(&header)),
            Some(("yandex".to_owned(), "s3cret".to_owned()))
        );
        assert_eq!(basic_credentials(Some("Bearer x")), None);
        assert_eq!(basic_credentials(None), None);
    }

    #[test]
    fn the_redirect_carries_code_state_client_and_scope() {
        let url = redirect_with_code(
            "https://social.yandex.net/broker/redirect",
            "AUTH",
            "abc 1",
            "yandex",
            "read",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://social.yandex.net/broker/redirect?code=AUTH&state=abc+1&client_id=yandex&scope=read"
        );
    }
}
