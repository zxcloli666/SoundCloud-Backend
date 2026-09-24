use catalog_sources::SourceError;

#[derive(Debug, thiserror::Error)]
pub enum EnrichError {
    #[error("enrichment database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("external source failed: {0}")]
    Source(#[from] SourceError),

    #[error("{0}")]
    Rejected(String),
}

pub type EnrichResult<T> = Result<T, EnrichError>;

impl EnrichError {
    pub fn rejected(message: impl Into<String>) -> Self {
        Self::Rejected(message.into())
    }
}
