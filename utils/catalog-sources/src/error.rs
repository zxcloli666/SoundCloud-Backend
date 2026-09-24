#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("external source is unreachable: {0}")]
    Unreachable(String),

    #[error("external source returned {status}: {body}")]
    Status {
        status: u16,
        body: String,
        retry_after_seconds: Option<u64>,
    },

    #[error("external source response exceeds {limit} bytes")]
    Oversized { limit: usize },

    #[error("external source payload is invalid: {0}")]
    Invalid(String),

    #[error("external source is not configured: {0}")]
    NotConfigured(&'static str),
}

pub type SourceResult<T> = Result<T, SourceError>;

impl SourceError {
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Self::Status {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
            _ => None,
        }
    }

    pub fn is_hard_client_error(&self) -> bool {
        matches!(
            self,
            Self::Status { status, .. }
                if (400..500).contains(status) && !matches!(status, 429 | 408 | 425)
        )
    }
}
