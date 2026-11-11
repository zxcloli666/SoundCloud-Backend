use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("LLM config {path} is unreadable: {reason}")]
    Unreadable { path: String, reason: String },
    #[error("LLM config is not valid: {0}")]
    Syntax(String),
    #[error("LLM config: {0}")]
    Invalid(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum LadderError {
    #[error("no provider of the ladder is available")]
    Unavailable,
    #[error("every provider of the ladder failed before an answer began")]
    Exhausted,
    #[error("the provider failed after its answer began")]
    ProviderFailed,
    #[error("the ladder deadline passed")]
    Deadline,
}
