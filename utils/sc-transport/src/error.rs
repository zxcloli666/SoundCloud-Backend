use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ScError {
    #[error("SoundCloud API error (status {status})")]
    Api {
        status: u16,
        body: Value,
        retry_after_sec: Option<i64>,
    },

    #[error("SoundCloud API unreachable: {0}")]
    Unreachable(String),

    #[error("SoundCloud response is invalid: {0}")]
    Invalid(String),
}

pub type ScResult<T> = Result<T, ScError>;

impl ScError {
    pub fn unreachable(detail: impl Into<String>) -> Self {
        Self::Unreachable(detail.into())
    }

    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid(detail.into())
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Api { status, .. } => match status {
                429 => "rate_limited",
                401 | 403 => "denied",
                404 | 410 => "not_found",
                500.. => "server_error",
                _ => "client_error",
            },
            Self::Unreachable(_) => "unreachable",
            Self::Invalid(_) => "invalid",
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}
