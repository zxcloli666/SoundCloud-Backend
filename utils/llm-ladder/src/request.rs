use std::time::Duration;

use serde::Serialize;

use crate::schema::Schema;

pub(crate) const BETWEEN_SECTIONS: &str = "\n\n";

#[derive(Clone, Debug)]
pub struct Request {
    pub system: System,
    pub messages: Vec<Message>,
    pub schema: Schema,
    pub budget_user: Option<String>,
    pub deadline: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct System {
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub text: String,
    pub ends_shared_prefix: bool,
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
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
}

impl System {
    pub fn fixed(text: impl Into<String>) -> Self {
        Self {
            sections: vec![Section::shared(text)],
        }
    }

    pub fn joined(&self) -> String {
        let texts: Vec<&str> = self
            .sections
            .iter()
            .map(|section| section.text.as_str())
            .filter(|text| !text.is_empty())
            .collect();
        texts.join(BETWEEN_SECTIONS)
    }
}

impl Section {
    pub fn shared(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ends_shared_prefix: true,
        }
    }

    pub fn own(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ends_shared_prefix: false,
        }
    }
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
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn is_unknown(&self) -> bool {
        self.prompt_tokens() == 0 && self.output_tokens == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sections_of_a_system_text_join_with_a_blank_line() {
        let system = System {
            sections: vec![
                Section::shared("rules"),
                Section::shared(""),
                Section::shared("character"),
                Section::own("state"),
            ],
        };
        assert_eq!(system.joined(), "rules\n\ncharacter\n\nstate");
        assert_eq!(System::fixed("rules").joined(), "rules");
        assert!(System::fixed("rules").sections[0].ends_shared_prefix);
        assert_eq!(System::default().joined(), "");
    }

    #[test]
    fn the_prompt_is_the_three_input_counts_together() {
        let usage = Usage {
            input_tokens: 40,
            cache_read_tokens: 3_000,
            cache_write_tokens: 200,
            output_tokens: 70,
        };
        assert_eq!(usage.prompt_tokens(), 3_240);
        assert!(!usage.is_unknown() && Usage::default().is_unknown());
        let cached_only = Usage {
            cache_read_tokens: 1,
            ..Usage::default()
        };
        assert!(!cached_only.is_unknown());
    }
}
