use llm_ladder::{Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::tools::{ToolSpec, Tools, is_name};
use crate::config::VoiceChatCfg;

const MAX_PERSONA_CHARS: usize = 64;
const MAX_HISTORY_ITEM_CHARS: usize = 2_000;
const MAX_NOTE_CHARS: usize = 300;
const MAX_BLOCK_BYTES: usize = 8 * 1024;
const MAX_RESULTS: usize = 16;
const JSON_BYTES_PER_COUNTED: usize = 4;
const BODY_ENVELOPE_BYTES: usize = 4 * 1024;

#[derive(Deserialize)]
pub struct ChatBody {
    #[serde(default)]
    text: String,
    #[serde(default)]
    persona: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    context: Value,
    #[serde(default)]
    history: Vec<Said>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    tools: Vec<ToolSpec>,
    #[serde(default)]
    results: Vec<ToolResult>,
    #[serde(default)]
    quiet: bool,
}

#[derive(Deserialize)]
struct Said {
    role: Speaker,
    #[serde(default)]
    text: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Speaker {
    User,
    Assistant,
}

#[derive(Deserialize, Serialize)]
struct ToolResult {
    name: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    result: Value,
}

pub struct Turn {
    pub text: String,
    pub persona: String,
    pub language: Option<String>,
    pub context: Option<String>,
    pub history: Vec<Message>,
    pub notes: Vec<String>,
    pub tools: Tools,
    pub results: Option<String>,
    pub quiet: bool,
}

impl ChatBody {
    pub fn into_turn(self, limits: &VoiceChatCfg) -> Result<Turn, String> {
        let text = self.text.trim();
        if text.is_empty() {
            return Err("text is empty".to_owned());
        }
        if text.chars().count() > limits.max_text_chars {
            return Err(format!(
                "text is longer than {} characters",
                limits.max_text_chars
            ));
        }
        if self.persona.chars().count() > MAX_PERSONA_CHARS {
            return Err(format!(
                "persona is longer than {MAX_PERSONA_CHARS} characters"
            ));
        }
        let tools = Tools::read(self.tools, limits.max_tools)?;
        let rounds_behind = self
            .results
            .iter()
            .filter(|result| tools.returns(&result.name))
            .count();
        let last_round = rounds_behind + 1 >= limits.max_tool_rounds;
        Ok(Turn {
            text: text.to_owned(),
            persona: self.persona,
            language: language(&self.language)?,
            context: context(self.context)?,
            history: history(self.history, limits.max_history),
            notes: notes(self.notes, limits.max_notes),
            tools: if last_round {
                tools.without_returning()
            } else {
                tools
            },
            quiet: self.quiet && self.results.is_empty(),
            results: results(self.results)?,
        })
    }
}

impl From<Speaker> for Role {
    fn from(speaker: Speaker) -> Self {
        match speaker {
            Speaker::User => Self::User,
            Speaker::Assistant => Self::Assistant,
        }
    }
}

fn language(code: &str) -> Result<Option<String>, String> {
    let code = code.trim();
    if code.is_empty() {
        return Ok(None);
    }
    let mut parts = code.split(['-', '_']);
    let primary = parts.next().unwrap_or_default();
    let known = (2..=3).contains(&primary.len())
        && primary.chars().all(|symbol| symbol.is_ascii_alphabetic())
        && parts.clone().count() <= 2
        && parts.all(|part| {
            (2..=8).contains(&part.len())
                && part.chars().all(|symbol| symbol.is_ascii_alphanumeric())
        });
    match known {
        true => Ok(Some(code.to_owned())),
        false => Err("language is not a language code".to_owned()),
    }
}

fn context(context: Value) -> Result<Option<String>, String> {
    match context {
        Value::Null => Ok(None),
        Value::Object(fields) if fields.is_empty() => Ok(None),
        Value::Object(fields) => block("context", &fields).map(Some),
        _ => Err("context must be an object".to_owned()),
    }
}

fn history(items: Vec<Said>, most: usize) -> Vec<Message> {
    let older = items.len().saturating_sub(most);
    let mut history: Vec<Message> = Vec::with_capacity(items.len() - older);
    for item in items.into_iter().skip(older) {
        let text = clipped(item.text.trim(), MAX_HISTORY_ITEM_CHARS);
        let role = Role::from(item.role);
        let opens_with_a_reply = history.is_empty() && role == Role::Assistant;
        if text.is_empty() || opens_with_a_reply {
            continue;
        }
        match history.last_mut().filter(|last| last.role == role) {
            Some(last) => {
                last.content.push('\n');
                last.content.push_str(text);
            }
            None => history.push(Message {
                role,
                content: text.to_owned(),
            }),
        }
    }
    history
}

fn notes(notes: Vec<String>, most: usize) -> Vec<String> {
    let notes: Vec<String> = notes
        .iter()
        .map(|note| note.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|note| !note.is_empty())
        .collect();
    let older = notes.len().saturating_sub(most);
    notes
        .iter()
        .skip(older)
        .map(|note| clipped(note, MAX_NOTE_CHARS).to_owned())
        .collect()
}

fn clipped(text: &str, most: usize) -> &str {
    match text.char_indices().nth(most) {
        Some((end, _)) => text[..end].trim_end(),
        None => text,
    }
}

fn results(results: Vec<ToolResult>) -> Result<Option<String>, String> {
    if results.is_empty() {
        return Ok(None);
    }
    if results.len() > MAX_RESULTS {
        return Err(format!("results has more than {MAX_RESULTS} items"));
    }
    if results.iter().any(|result| !is_name(&result.name)) {
        return Err("a result names something that is not a tool".to_owned());
    }
    block("results", &results).map(Some)
}

fn block(name: &str, value: &impl Serialize) -> Result<String, String> {
    let text = serde_json::to_string(value).map_err(|_| format!("{name} is not JSON"))?;
    match text.len() > MAX_BLOCK_BYTES {
        true => Err(format!("{name} is larger than 8 KiB")),
        false => Ok(text),
    }
}

pub fn body_limit(limits: &VoiceChatCfg) -> usize {
    let counted = limits.max_text_chars
        + MAX_PERSONA_CHARS
        + limits.max_history * MAX_HISTORY_ITEM_CHARS
        + limits.max_notes * MAX_NOTE_CHARS
        + 2 * MAX_BLOCK_BYTES
        + limits.max_system_bytes;
    counted * JSON_BYTES_PER_COUNTED + BODY_ENVELOPE_BYTES
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
