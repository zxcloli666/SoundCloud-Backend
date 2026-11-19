#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("the mobile HTTP client could not be built: {0}")]
    Client(String),

    #[error("the mobile request failed in transport: {0}")]
    Transport(String),

    #[error("the mobile response is larger than {0} bytes")]
    TooLarge(usize),

    #[error("the mobile request target is not a mobile API host")]
    ForeignHost,

    #[error("the mobile request could not be built: {0}")]
    Invalid(&'static str),

    #[error("the system random source failed")]
    Random,

    #[error("the token endpoint answered HTTP {status} ({code})", code = .error.as_deref().unwrap_or("no error code"))]
    Refused {
        status: u16,
        error: Option<String>,
        description: Option<String>,
    },

    #[error("the token endpoint answered without an access token")]
    InvalidTokenResponse,
}

impl MobileError {
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    pub(crate) fn transport(error: wreq::Error) -> Self {
        Self::Transport(error.without_uri().to_string())
    }
}
