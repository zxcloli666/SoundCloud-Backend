use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diag::{App, ErrorKind, Session};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Report {
    pub header: Header,
    pub counts: Counts,
    pub offset_ms: Option<i64>,
    pub timeline: Vec<TimelineEntry>,
    pub errors: Vec<ReportError>,
    pub hints: Vec<Hint>,
    pub missing: Missing,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Header {
    pub view: View,
    pub code: String,
    pub created: String,
    pub app: ReportApp,
    pub session: Session,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReportApp {
    pub version: String,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub install: String,
    pub locale: String,
    pub id: Option<String>,
}

impl ReportApp {
    pub fn from_app(app: &App) -> Self {
        Self {
            version: app.version.clone(),
            os: app.os.clone(),
            os_version: app.os_version.clone(),
            arch: app.arch.clone(),
            install: app.install.clone(),
            locale: app.locale.clone(),
            id: Some(app.id.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Internal,
    Public,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Counts {
    pub attempts: u32,
    pub failed_attempts: u32,
    pub errors: u32,
    pub traces_wanted: u32,
    pub traces_found: u32,
    pub server_spans: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    Client,
    Server,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct TimelineEntry {
    pub at: String,
    pub who: Who,
    pub host: String,
    pub service: String,
    pub name: String,
    pub outcome: Outcome,
    pub status: Option<u16>,
    pub ms: u64,
    pub message: Option<String>,
    pub count: u32,
    pub attrs: BTreeMap<String, String>,
    pub trace: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReportError {
    pub at: String,
    pub kind: ErrorKind,
    pub message: String,
    pub count: u32,
    pub fingerprint: String,
    pub context: BTreeMap<String, String>,
    pub component_stack: Option<String>,
    pub frames: Vec<Frame>,
    pub issue: Option<IssueStats>,
    pub trace: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Frame {
    pub function: Option<String>,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub source: Option<String>,
    pub source_line: Option<u32>,
    pub source_column: Option<u32>,
    pub symbol: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct IssueStats {
    pub events: i64,
    pub installs: i64,
    pub installs_24h: i64,
    pub first_at: String,
    pub last_at: String,
    pub first_version: String,
    pub versions: BTreeMap<String, i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    RelayBlocked,
    OriginBlocked,
    AllRoutesDown,
    NeverReached,
    OurServerError,
    OurSlowness,
    TierMissing,
    RegionBlocked,
    SessionExpired,
    Busy,
    Widespread,
    LocalOnly,
    NewInVersion,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Hint {
    pub rule: Rule,
    pub text: String,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Evidence {
    pub timeline: Vec<u32>,
    pub errors: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourcemapState {
    NotNeeded,
    Applied,
    Missing,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Missing {
    pub traces: u32,
    pub trace_ids: Vec<String>,
    pub sourcemaps: SourcemapState,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Issue {
    pub fingerprint: String,
    pub kind: String,
    pub title: String,
    pub events: i64,
    pub installs: i64,
    pub first_at: String,
    pub last_at: String,
    pub first_version: String,
    pub versions: BTreeMap<String, i64>,
    pub daily: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct IssueDay {
    pub day: String,
    pub events: i64,
    pub installs: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct IssueEvent {
    pub at: String,
    pub app_version: String,
    pub os: String,
    pub trace: Option<String>,
    pub context: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct IssueDetail {
    #[serde(flatten)]
    pub issue: Issue,
    pub days: Vec<IssueDay>,
    pub events: Vec<IssueEvent>,
}
