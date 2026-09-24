use catalog_sources::SourceError;

#[derive(Debug, thiserror::Error)]
pub enum CrawlError {
    #[error("crawl database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("external source failed: {0}")]
    Source(#[from] SourceError),

    #[error("artist crawl exceeded its deadline")]
    Deadline,
}

pub type CrawlResult<T = ()> = Result<T, CrawlError>;

impl CrawlError {
    pub fn is_payload_failure(&self) -> bool {
        matches!(self, Self::Source(SourceError::Invalid(_)))
    }
}
