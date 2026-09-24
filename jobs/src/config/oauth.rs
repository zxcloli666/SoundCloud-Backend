use super::ConfigError;
use super::env;

#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub token_url: url::Url,
    pub bootstrap_app: Option<OAuthAppBootstrap>,
}

#[derive(Clone, Debug)]
pub struct OAuthAppBootstrap {
    pub name: String,
    pub client_id: String,
    pub client_secret: redact::Secret<String>,
    pub redirect_uri: String,
}

impl OAuthConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let token_url = env::optional("SOUNDCLOUD_TOKEN_URL")
            .unwrap_or_else(|| "https://secure.soundcloud.com/oauth/token".to_owned())
            .parse::<url::Url>()
            .map_err(|error| ConfigError::Invalid {
                key: "SOUNDCLOUD_TOKEN_URL".to_owned(),
                reason: error.to_string(),
            })?;
        if token_url.scheme() != "https" {
            return Err(ConfigError::Invalid {
                key: "SOUNDCLOUD_TOKEN_URL".to_owned(),
                reason: "must use https".to_owned(),
            });
        }
        let bootstrap_app = OAuthAppBootstrap::from_env()?;
        Ok(Self {
            token_url,
            bootstrap_app,
        })
    }
}

impl OAuthAppBootstrap {
    pub(crate) fn from_env() -> Result<Option<Self>, ConfigError> {
        let app = match (
            env::optional("SOUNDCLOUD_CLIENT_ID"),
            env::optional("SOUNDCLOUD_CLIENT_SECRET"),
        ) {
            (None, None) => None,
            (Some(_), None) => {
                return Err(ConfigError::Missing("SOUNDCLOUD_CLIENT_SECRET".to_owned()));
            }
            (None, Some(_)) => {
                return Err(ConfigError::Missing("SOUNDCLOUD_CLIENT_ID".to_owned()));
            }
            (Some(client_id), Some(client_secret)) => {
                let redirect_uri = env::optional("SOUNDCLOUD_REDIRECT_URI")
                    .unwrap_or_else(|| "http://localhost:3000/auth/callback".to_owned());
                Some(Self::new(
                    env::optional("SOUNDCLOUD_OAUTH_APP_NAME")
                        .unwrap_or_else(|| "default".to_owned()),
                    client_id,
                    client_secret,
                    redirect_uri,
                )?)
            }
        };
        Ok(app)
    }

    pub(super) fn new(
        name: String,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    ) -> Result<Self, ConfigError> {
        let name = non_blank("SOUNDCLOUD_OAUTH_APP_NAME", name)?;
        let client_id = non_blank("SOUNDCLOUD_CLIENT_ID", client_id)?;
        if client_secret.trim().is_empty() {
            return Err(ConfigError::Invalid {
                key: "SOUNDCLOUD_CLIENT_SECRET".to_owned(),
                reason: "must not be blank".to_owned(),
            });
        }
        let redirect_uri = non_blank("SOUNDCLOUD_REDIRECT_URI", redirect_uri)?;
        url::Url::parse(&redirect_uri).map_err(|error| ConfigError::Invalid {
            key: "SOUNDCLOUD_REDIRECT_URI".to_owned(),
            reason: error.to_string(),
        })?;
        Ok(Self {
            name,
            client_id,
            client_secret: client_secret.into(),
            redirect_uri,
        })
    }

    pub(crate) fn id(&self) -> uuid::Uuid {
        const NAMESPACE: uuid::Uuid =
            uuid::Uuid::from_u128(0xa29a_24d2_344e_5daa_a627_0a3f_2d38_c7a1);
        uuid::Uuid::new_v5(&NAMESPACE, self.client_id.as_bytes())
    }
}

fn non_blank(key: &str, value: String) -> Result<String, ConfigError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must not be blank".to_owned(),
        });
    }
    Ok(value.to_owned())
}
