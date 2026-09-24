use std::fmt::{Debug, Formatter};
use std::time::Duration;

use super::ConfigError;
use super::env;

#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub tls: TlsConfig,
    pub fast_pool: PoolConfig,
    pub bulk_pool: PoolConfig,
}

impl Debug for DatabaseConfig {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DatabaseConfig")
            .field("url", &redact_url(&self.url))
            .field("tls", &self.tls)
            .field("fast_pool", &self.fast_pool)
            .field("bulk_pool", &self.bulk_pool)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct TlsConfig {
    pub mode: Option<String>,
    pub root_certificate: Option<String>,
    pub client_certificate: Option<String>,
    pub client_key: Option<String>,
    pub require_mtls: bool,
}

#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub minimum: u32,
    pub maximum: u32,
    pub acquire_timeout: Duration,
    pub session: SessionLimits,
}

#[derive(Clone, Debug)]
pub struct SessionLimits {
    pub statement_timeout: Duration,
    pub lock_timeout: Duration,
    pub idle_transaction_timeout: Duration,
}

impl DatabaseConfig {
    pub fn from_env(prefix: &str, mtls_by_default: bool) -> Result<Self, ConfigError> {
        let key = |name: &str| format!("{prefix}{name}");
        let url = match env::optional(&key("DATABASE_URL")) {
            Some(url) => url,
            None => compose_url(
                &env::required(&key("DATABASE_USERNAME"))?,
                &env::required(&key("DATABASE_PASSWORD"))?,
                &env::required(&key("DATABASE_HOST"))?,
                env::parse(&key("DATABASE_PORT"), "5432")?,
                &env::required(&key("DATABASE_NAME"))?,
            ),
        };

        let require_mtls = env::boolean(&key("DATABASE_REQUIRE_MTLS"), mtls_by_default)?;
        let from_url = TlsConfig::from_url(&url, require_mtls, prefix)?;
        let configured_mode = env::optional(&key("DATABASE_SSL_MODE"));
        let configured_root = env::optional(&key("DATABASE_SSL_CA"));
        let configured_certificate = env::optional(&key("DATABASE_SSL_CERT"));
        let configured_key = env::optional(&key("DATABASE_SSL_KEY"));
        let configured_tls_material = configured_root.is_some()
            || configured_certificate.is_some()
            || configured_key.is_some();
        let tls = TlsConfig {
            mode: configured_mode.or_else(|| {
                configured_tls_material
                    .then(|| "verify-full".to_owned())
                    .or(from_url.mode)
            }),
            root_certificate: configured_root.or(from_url.root_certificate),
            client_certificate: configured_certificate.or(from_url.client_certificate),
            client_key: configured_key.or(from_url.client_key),
            require_mtls,
        };
        tls.validate(prefix)?;

        Ok(Self {
            url,
            tls,
            fast_pool: PoolConfig {
                minimum: env::parse(&key("PG_POOL_MIN"), "2")?,
                maximum: env::parse(&key("PG_POOL_MAX"), "8")?,
                acquire_timeout: Duration::from_millis(env::positive_u64(
                    &key("PG_ACQUIRE_TIMEOUT_MS"),
                    2_000,
                )?),
                session: SessionLimits {
                    statement_timeout: Duration::from_secs(30),
                    lock_timeout: Duration::from_secs(1),
                    idle_transaction_timeout: Duration::from_secs(15),
                },
            },
            bulk_pool: PoolConfig {
                minimum: env::parse(&key("PG_BULK_POOL_MIN"), "0")?,
                maximum: env::parse(&key("PG_BULK_POOL_MAX"), "2")?,
                acquire_timeout: Duration::from_millis(env::positive_u64(
                    &key("PG_BULK_ACQUIRE_TIMEOUT_MS"),
                    5_000,
                )?),
                session: SessionLimits {
                    statement_timeout: Duration::from_secs(570),
                    lock_timeout: Duration::from_secs(1),
                    idle_transaction_timeout: Duration::from_secs(15),
                },
            },
        })
    }
}

impl TlsConfig {
    fn from_url(url: &str, require_mtls: bool, prefix: &str) -> Result<Self, ConfigError> {
        let parsed = url::Url::parse(url).map_err(|error| ConfigError::Invalid {
            key: format!("{prefix}DATABASE_URL"),
            reason: format!("invalid PostgreSQL URL: {error}"),
        })?;
        let parameter = |name: &str| {
            parsed
                .query_pairs()
                .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
        };
        Ok(Self {
            mode: parameter("sslmode"),
            root_certificate: parameter("sslrootcert"),
            client_certificate: parameter("sslcert"),
            client_key: parameter("sslkey"),
            require_mtls,
        })
    }

    fn validate(&self, prefix: &str) -> Result<(), ConfigError> {
        let client_certificate = self.client_certificate.is_some();
        let client_key = self.client_key.is_some();
        if client_certificate != client_key {
            return Err(ConfigError::Invalid {
                key: format!("{prefix}DATABASE_SSL_*"),
                reason: "client certificate and client key must be supplied together".to_owned(),
            });
        }

        let any_tls_material = self.root_certificate.is_some() || client_certificate;
        if any_tls_material && !matches!(self.mode.as_deref(), Some("verify-ca" | "verify-full")) {
            return Err(ConfigError::Invalid {
                key: format!("{prefix}DATABASE_SSL_MODE"),
                reason: "certificate material requires verify-ca or verify-full".to_owned(),
            });
        }

        if self.require_mtls
            && (self.mode.as_deref() != Some("verify-full")
                || self.root_certificate.is_none()
                || !client_certificate)
        {
            return Err(ConfigError::Invalid {
                key: format!("{prefix}DATABASE_SSL_*"),
                reason: "mTLS requires verify-full, CA, client certificate, and client key"
                    .to_owned(),
            });
        }

        Ok(())
    }
}

fn compose_url(user: &str, password: &str, host: &str, port: u16, database: &str) -> String {
    format!(
        "postgres://{}:{}@{host}:{port}/{}",
        urlencoding::encode(user),
        urlencoding::encode(password),
        urlencoding::encode(database),
    )
}

fn redact_url(url: &str) -> String {
    let Some((scheme, remainder)) = url.split_once("://") else {
        return "<redacted>".to_owned();
    };
    let Some((credentials, address)) = remainder.split_once('@') else {
        return format!("{scheme}://<redacted>");
    };
    let username = credentials
        .split_once(':')
        .map_or(credentials, |(username, _)| username);
    format!("{scheme}://{username}:***@{address}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_redacts_password() {
        assert_eq!(
            redact_url("postgres://sound:secret@db:5432/main"),
            "postgres://sound:***@db:5432/main"
        );
    }

    #[test]
    fn partial_mtls_is_rejected() {
        let tls = TlsConfig {
            mode: Some("verify-full".to_owned()),
            root_certificate: Some("/ca".to_owned()),
            client_certificate: None,
            client_key: Some("/key".to_owned()),
            require_mtls: true,
        };

        assert!(tls.validate("").is_err());
    }

    #[test]
    fn url_mtls_parameters_are_validated_as_effective_configuration() {
        let tls = TlsConfig::from_url(
            "postgres://jobs@db/main?sslmode=verify-full&sslrootcert=%2Ftls%2Fca.pem&sslcert=%2Ftls%2Fclient.pem&sslkey=%2Ftls%2Fclient.key",
            true,
            "",
        )
        .unwrap();

        assert!(tls.validate("").is_ok());
    }

    #[test]
    fn one_way_tls_accepts_a_ca_without_client_credentials() {
        let tls = TlsConfig {
            mode: Some("verify-full".to_owned()),
            root_certificate: Some("/ca".to_owned()),
            client_certificate: None,
            client_key: None,
            require_mtls: false,
        };

        assert!(tls.validate("").is_ok());
    }

    #[test]
    fn certificate_material_cannot_be_silently_disabled() {
        let tls = TlsConfig {
            mode: Some("disable".to_owned()),
            root_certificate: Some("/ca".to_owned()),
            client_certificate: None,
            client_key: None,
            require_mtls: false,
        };

        assert!(tls.validate("").is_err());
    }
}
