use backend_contracts::admin_catalog::AdminCatalogReply;
use serde::Serialize;
use serde_json::{Value, json};
use wt::Raced;

pub struct Refusal {
    status: u16,
    message: String,
}

impl Refusal {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, message)
    }

    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn into_reply(self) -> AdminCatalogReply {
        let reason = match self.status {
            400 => "Bad Request",
            404 => "Not Found",
            409 => "Conflict",
            _ => "Error",
        };
        AdminCatalogReply {
            status: self.status,
            body: json!({
                "statusCode": self.status,
                "message": self.message,
                "error": reason,
            }),
        }
    }
}

pub enum Fail {
    Refused(Refusal),
    Database(sqlx::Error),
}

impl From<Refusal> for Fail {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<sqlx::Error> for Fail {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl Fail {
    pub fn retryable(&self) -> bool {
        let Self::Database(error) = self else {
            return false;
        };
        error.raced() || wt::conflicted(error)
    }
}

pub type Done = Result<Value, Fail>;

pub fn encode<T: Serialize>(value: &T) -> Result<Value, Fail> {
    serde_json::to_value(value)
        .map_err(|error| Fail::Database(sqlx::Error::Encode(Box::new(error))))
}
