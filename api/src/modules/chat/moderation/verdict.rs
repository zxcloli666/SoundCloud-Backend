use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Spam,
    Harassment,
    Doxxing,
    Illegal,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    None,
    Delete,
    Ban,
}
