use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const QUOTES_MAX: usize = 50;
pub const RATIONALE_CHARS_MAX: usize = 1_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub genuine: bool,
    pub category: Category,
    pub severity: Severity,
    pub recommended_action: RecommendedAction,
    pub confidence: f32,
    pub quote_seq: Vec<i64>,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Spam,
    Harassment,
    Doxxing,
    Illegal,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    None,
    Delete,
    Ban,
}

impl Verdict {
    pub fn parse(raw: &str) -> Option<Self> {
        let verdict: Verdict = serde_json::from_str(raw.trim()).ok()?;
        verdict.well_formed().then_some(verdict)
    }

    fn well_formed(&self) -> bool {
        self.confidence.is_finite()
            && (0.0..=1.0).contains(&self.confidence)
            && self.quote_seq.len() <= QUOTES_MAX
            && self.rationale.chars().count() <= RATIONALE_CHARS_MAX
            && !(self.genuine && self.category == Category::None)
    }
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "genuine",
            "category",
            "severity",
            "recommended_action",
            "confidence",
            "quote_seq",
            "rationale"
        ],
        "properties": {
            "genuine": {"type": "boolean"},
            "category": {
                "type": "string",
                "enum": ["spam", "harassment", "doxxing", "illegal", "none"]
            },
            "severity": {"type": "string", "enum": ["low", "medium", "high"]},
            "recommended_action": {"type": "string", "enum": ["none", "delete", "ban"]},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1},
            "quote_seq": {
                "type": "array",
                "items": {"type": "integer"},
                "maxItems": QUOTES_MAX
            },
            "rationale": {"type": "string", "maxLength": 500}
        }
    })
}
