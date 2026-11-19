use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use redact::Secret;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;
use url::form_urlencoded;

use crate::app::MobileApp;
use crate::client::{Caller, MobileClient, MobileReply};
use crate::error::MobileError;
use crate::request::MobileRequest;

pub const WEB_AUTH_URL: &str = "https://secure.soundcloud.com/web-auth";
pub const TOKEN_URL: &str = "https://api-auth.soundcloud.com/oauth/token";
pub const REDIRECT_URI: &str = "sc://auth";

const LOCALE: &str = "en";
const VERIFIER_BYTES: usize = 32;
const STATE_BYTES: usize = 16;

pub struct Pkce {
    verifier: Secret<String>,
    challenge: String,
}

impl Pkce {
    pub fn generate() -> Result<Self, MobileError> {
        let verifier = random_token(VERIFIER_BYTES)?;
        let challenge = s256_challenge(&verifier);
        Ok(Self {
            verifier: Secret::new(verifier),
            challenge,
        })
    }

    pub fn verifier(&self) -> &Secret<String> {
        &self.verifier
    }

    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

#[derive(Clone, Debug)]
pub struct TokenSet {
    pub access_token: Secret<String>,
    pub refresh_token: Option<Secret<String>>,
    pub expires_in: Option<i64>,
    pub scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: Option<Secret<String>>,
    refresh_token: Option<Secret<String>>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct RefusalBody {
    error: Option<String>,
    error_description: Option<String>,
}

pub fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn new_state() -> Result<Secret<String>, MobileError> {
    random_token(STATE_BYTES).map(Secret::new)
}

pub fn new_device_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn client_authorization(app: &MobileApp) -> Secret<String> {
    let pair = Secret::new(format!("{}:{}", app.client_id, app.client_secret.expose()));
    Secret::new(URL_SAFE_NO_PAD.encode(Sha256::digest(pair.expose().as_bytes())))
}

pub fn authorize_url(
    app: &MobileApp,
    challenge: &str,
    state: &str,
    device_id: &str,
) -> Result<Url, MobileError> {
    Url::parse_with_params(
        WEB_AUTH_URL,
        [
            ("client_id", app.client_id.as_str()),
            ("app_id", app.app_id.as_str()),
            ("device_id", device_id),
            ("start_view", "sign_in"),
            ("redirect_uri", REDIRECT_URI),
            ("response_type", "code"),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("ui_evo", "true"),
            ("stand_alone", "true"),
            ("tracking", "local"),
            ("show_confirmation", "true"),
            ("theme", "dark"),
            ("locale", LOCALE),
            ("state", state),
        ],
    )
    .map_err(|_| MobileError::Invalid("the web-auth URL does not parse"))
}

impl MobileClient {
    pub async fn exchange_code(
        &self,
        code: &Secret<String>,
        verifier: &Secret<String>,
        device_id: &str,
    ) -> Result<TokenSet, MobileError> {
        let form = form(&[
            ("grant_type", "authorization_code"),
            ("client_id", self.app().client_id.as_str()),
            ("code", code.expose()),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier.expose()),
        ]);
        self.token(form, device_id).await
    }

    pub async fn refresh(
        &self,
        refresh_token: &Secret<String>,
        device_id: &str,
    ) -> Result<TokenSet, MobileError> {
        let form = form(&[
            ("grant_type", "refresh_token"),
            ("client_id", self.app().client_id.as_str()),
            ("refresh_token", refresh_token.expose()),
        ]);
        self.token(form, device_id).await
    }

    async fn token(&self, form: String, device_id: &str) -> Result<TokenSet, MobileError> {
        let reply = self
            .call(&Caller::device(device_id), &MobileRequest::token(form))
            .await?;
        token_set(&reply)
    }
}

fn token_set(reply: &MobileReply) -> Result<TokenSet, MobileError> {
    if reply.is_captcha() {
        return Err(MobileError::Captcha);
    }
    if !(200..300).contains(&reply.status) {
        let refusal = serde_json::from_slice::<RefusalBody>(&reply.body).ok();
        return Err(MobileError::Refused {
            status: reply.status,
            error: refusal.as_ref().and_then(|body| body.error.clone()),
            description: refusal.and_then(|body| body.error_description),
        });
    }
    let body = serde_json::from_slice::<TokenBody>(&reply.body)
        .map_err(|_| MobileError::InvalidTokenResponse)?;
    let access_token = filled(body.access_token).ok_or(MobileError::InvalidTokenResponse)?;
    Ok(TokenSet {
        access_token,
        refresh_token: filled(body.refresh_token),
        expires_in: body.expires_in,
        scope: body.scope.filter(|scope| !scope.trim().is_empty()),
    })
}

fn filled(token: Option<Secret<String>>) -> Option<Secret<String>> {
    token.filter(|token| !token.expose().trim().is_empty())
}

fn form(fields: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish()
}

fn random_token(bytes: usize) -> Result<String, MobileError> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|_| MobileError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bytes::Bytes;

    use super::*;

    fn dummy_app() -> MobileApp {
        MobileApp {
            client_id: "dummy-client".to_owned(),
            client_secret: Secret::new("dummy-secret".to_owned()),
            app_id: "dummy-app".to_owned(),
            app_version: "dummy-version".to_owned(),
            user_agent: "dummy-agent".to_owned(),
        }
    }

    #[test]
    fn the_web_auth_url_carries_every_sign_in_parameter() {
        let Ok(url) = authorize_url(
            &dummy_app(),
            "dummy-challenge",
            "dummy-state",
            "dummy-device",
        ) else {
            panic!("the web-auth URL does not build");
        };
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();

        assert_eq!(url.host_str(), Some("secure.soundcloud.com"));
        assert_eq!(url.path(), "/web-auth");
        for (name, value) in [
            ("client_id", "dummy-client"),
            ("app_id", "dummy-app"),
            ("device_id", "dummy-device"),
            ("start_view", "sign_in"),
            ("redirect_uri", "sc://auth"),
            ("response_type", "code"),
            ("code_challenge", "dummy-challenge"),
            ("code_challenge_method", "S256"),
            ("ui_evo", "true"),
            ("stand_alone", "true"),
            ("tracking", "local"),
            ("show_confirmation", "true"),
            ("theme", "dark"),
            ("locale", "en"),
            ("state", "dummy-state"),
        ] {
            assert_eq!(query.get(name).map(String::as_str), Some(value), "{name}");
        }
        assert_eq!(query.len(), 15);
        assert!(!url.as_str().contains("dummy-secret"));
    }

    #[test]
    fn the_challenge_is_the_rfc_7636_s256_of_the_verifier() {
        assert_eq!(
            s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_generated_pair_matches_and_never_repeats() {
        let (Ok(first), Ok(second)) = (Pkce::generate(), Pkce::generate()) else {
            panic!("the random source failed");
        };

        assert_eq!(first.verifier().expose().len(), 43);
        assert_eq!(first.challenge(), s256_challenge(first.verifier().expose()));
        assert_ne!(first.verifier().expose(), second.verifier().expose());
        assert_eq!(new_device_id().len(), 32);
        assert_ne!(new_device_id(), new_device_id());
    }

    #[test]
    fn the_client_header_is_the_unpadded_url_safe_sha256_of_id_and_secret() {
        let digest = Sha256::digest(b"dummy-client:dummy-secret");
        let by_hand = base64::engine::general_purpose::STANDARD
            .encode(digest)
            .replace('+', "-")
            .replace('/', "_")
            .trim_end_matches('=')
            .to_owned();

        let header = client_authorization(&dummy_app());

        assert_eq!(header.expose(), &by_hand);
        assert_eq!(
            header.expose(),
            "Fl8tNnGSPPlpfphjw9v18PGZXLT9tZysdaCgWskX5uo"
        );
    }

    #[test]
    fn a_token_set_prints_no_token() {
        let reply = MobileReply {
            status: 200,
            plan: None,
            location: None,
            body: Bytes::from_static(
                br#"{"access_token":"dummy-access","refresh_token":"dummy-refresh","expires_in":3600,"scope":"*","token_type":"bearer"}"#,
            ),
        };
        let Ok(tokens) = token_set(&reply) else {
            panic!("the token body does not parse");
        };

        let printed = format!("{tokens:?} {tokens:#?}");

        assert!(!printed.contains("dummy-access"), "{printed}");
        assert!(!printed.contains("dummy-refresh"), "{printed}");
        assert_eq!(tokens.access_token.expose(), "dummy-access");
        assert_eq!(tokens.expires_in, Some(3600));
    }

    #[test]
    fn a_refusal_keeps_the_oauth_error_and_a_missing_refresh_token_is_none() {
        let refused = MobileReply {
            status: 400,
            plan: None,
            location: None,
            body: Bytes::from_static(br#"{"error":"invalid_grant"}"#),
        };
        assert!(matches!(
            token_set(&refused),
            Err(MobileError::Refused { status: 400, error: Some(code), .. }) if code == "invalid_grant"
        ));

        let no_refresh = MobileReply {
            status: 200,
            plan: None,
            location: None,
            body: Bytes::from_static(br#"{"access_token":"a","refresh_token":""}"#),
        };
        assert!(token_set(&no_refresh).is_ok_and(|tokens| tokens.refresh_token.is_none()));
    }

    #[test]
    fn a_captcha_challenge_is_its_own_refusal() {
        let challenged = MobileReply {
            status: 403,
            plan: None,
            location: None,
            body: Bytes::from_static(
                br#"{"url":"https://geo.captcha-delivery.com/interstitial/?initialCid=x&cid=y"}"#,
            ),
        };

        assert!(matches!(token_set(&challenged), Err(MobileError::Captcha)));
    }

    #[test]
    fn the_token_form_is_url_encoded() {
        assert_eq!(
            form(&[("grant_type", "authorization_code"), ("code", "a b&c")]),
            "grant_type=authorization_code&code=a+b%26c"
        );
    }
}
