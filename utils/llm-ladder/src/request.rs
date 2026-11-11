use std::time::Duration;

use serde::Serialize;

use crate::schema::Schema;

pub(crate) const BETWEEN_SECTIONS: &str = "\n\n";
const BYTES_PER_TOKEN: usize = 3;

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

    pub(crate) fn shared_ends(&self) -> Vec<usize> {
        let mut ends = Vec::new();
        let mut length = 0;
        for section in &self.sections {
            if !section.text.is_empty() {
                let lead = match length {
                    0 => 0,
                    _ => BETWEEN_SECTIONS.len(),
                };
                length += lead + section.text.len();
            }
            if section.ends_shared_prefix && length > 0 && ends.last() != Some(&length) {
                ends.push(length);
            }
        }
        ends
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

impl Request {
    pub(crate) fn size(&self) -> usize {
        let messages: usize = self
            .messages
            .iter()
            .map(|message| message.content.len())
            .sum();
        let schema = serde_json::to_vec(&self.schema).map_or(0, |schema| schema.len());
        self.system.joined().len() + messages + schema
    }
}

impl Usage {
    pub(crate) fn estimated(sent_bytes: usize, received_bytes: usize) -> Self {
        let tokens = |bytes: usize| bytes.div_ceil(BYTES_PER_TOKEN) as u64;
        Self {
            input_tokens: tokens(sent_bytes),
            output_tokens: tokens(received_bytes),
            ..Self::default()
        }
    }

    pub(crate) fn completed_by(self, estimate: Self) -> Self {
        let prompt = match self.prompt_tokens() {
            0 => estimate,
            _ => self,
        };
        Self {
            output_tokens: self.output_tokens.max(estimate.output_tokens),
            ..prompt
        }
    }

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
    fn a_shared_prefix_ends_where_its_last_section_ends_in_the_joined_text() {
        let system = System {
            sections: vec![
                Section::shared("правила"),
                Section::shared(""),
                Section::own("character"),
                Section::shared("state"),
            ],
        };
        let joined = system.joined();
        assert_eq!(system.shared_ends(), [14, joined.len()]);
        assert_eq!(&joined[..14], "правила");
        assert_eq!(System::fixed("rules").shared_ends(), [5]);
        let own = System {
            sections: vec![Section::shared(""), Section::own("state")],
        };
        assert_eq!(own.shared_ends(), Vec::<usize>::new());
        assert_eq!(System::default().shared_ends(), Vec::<usize>::new());
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

    #[test]
    fn an_estimate_counts_a_token_for_every_few_bytes_rounded_up() {
        let request = Request {
            system: System::fixed("rules"),
            messages: vec![Message::user("hello"), Message::assistant("hi")],
            schema: Schema::String,
            budget_user: None,
            deadline: None,
        };
        assert_eq!(request.size(), 5 + 5 + 2 + "{\"type\":\"string\"}".len());
        assert_eq!(
            Usage::estimated(request.size(), 7),
            Usage {
                input_tokens: 10,
                output_tokens: 3,
                ..Usage::default()
            }
        );
        assert!(Usage::estimated(0, 0).is_unknown());
    }

    #[test]
    fn an_estimate_fills_only_what_the_provider_did_not_count() {
        let estimate = Usage::estimated(300, 30);
        assert_eq!(Usage::default().completed_by(estimate), estimate);
        let opening = Usage {
            input_tokens: 40,
            cache_read_tokens: 3_000,
            cache_write_tokens: 200,
            output_tokens: 1,
        };
        assert_eq!(
            opening.completed_by(estimate),
            Usage {
                output_tokens: 10,
                ..opening
            }
        );
        let wordy = Usage {
            output_tokens: 70,
            ..opening
        };
        assert_eq!(wordy.completed_by(estimate), wordy);
        let output_only = Usage {
            output_tokens: 70,
            ..Usage::default()
        };
        assert_eq!(
            output_only.completed_by(estimate),
            Usage {
                input_tokens: 100,
                output_tokens: 70,
                ..Usage::default()
            }
        );
    }
}
