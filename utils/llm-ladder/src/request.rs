use std::time::Duration;

use serde::Serialize;

use crate::schema::Schema;

#[derive(Clone, Debug)]
pub struct Request {
    pub system: String,
    pub messages: Vec<Message>,
    pub schema: Schema,
    pub budget_user: Option<String>,
    pub deadline: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

impl Usage {
    pub fn is_unknown(&self) -> bool {
        self.input_tokens == 0 && self.output_tokens == 0
    }
}
