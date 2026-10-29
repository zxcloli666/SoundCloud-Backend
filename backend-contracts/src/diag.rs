use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const CONTRACT_VERSION: u32 = 1;
pub const CONTRACT_PATH: &str = "contract/diag-contract.json";
pub const BUNDLE_KIND: &str = "scd-diagnostics";
pub const BUNDLE_VERSION: u8 = 1;
pub const MAX_BUNDLE_BYTES: usize = 256 * 1024;
pub const MAX_ATTEMPTS: usize = 300;
pub const MAX_ERRORS: usize = 50;
pub const MAX_LOG_LINES: usize = 150;
pub const MAX_EVENTS: usize = 50;
pub const MAX_CONTEXT_KEYS: usize = 10;

const SHORT: usize = 64;
const MESSAGE: usize = 2000;
const STACK: usize = 8000;
const COMPONENT_STACK: usize = 4000;
const PATH: usize = 1000;
const ERROR: usize = 1000;
const LOG_LINE: usize = 500;
const NETCHECK: usize = 8192;
const CONTEXT_VALUE: usize = 300;

const TRACE_PATTERN: &str = "^[0-9a-f]{32}$";
const SPAN_PATTERN: &str = "^[0-9a-f]{16}$";
const FINGERPRINT_PATTERN: &str = "^[0-9a-f]{16}$";
const INSTALL_PATTERN: &str = "^[0-9a-f]{12}$";

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    #[schemars(extend("const" = "scd-diagnostics"), length(max = 32))]
    pub kind: String,
    #[schemars(extend("const" = 1))]
    pub v: u8,
    #[schemars(length(max = 64))]
    pub at: String,
    pub app: App,
    pub session: Session,
    pub trigger: Trigger,
    #[schemars(length(max = 300))]
    pub attempts: Vec<Attempt>,
    #[schemars(length(max = 50))]
    pub errors: Vec<ErrorEntry>,
    #[schemars(length(max = 8192))]
    pub netcheck: Option<String>,
    #[schemars(length(max = 150), inner(length(max = 500)))]
    pub log_tail: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventsBody {
    pub app: App,
    #[schemars(length(min = 1, max = 50))]
    pub events: Vec<ErrorEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct App {
    #[schemars(length(max = 64))]
    pub version: String,
    #[schemars(length(max = 64))]
    pub os: String,
    #[schemars(length(max = 64))]
    pub os_version: String,
    #[schemars(length(max = 64))]
    pub arch: String,
    #[schemars(length(max = 64))]
    pub install: String,
    #[schemars(length(max = 64))]
    pub locale: String,
    #[schemars(pattern("^[0-9a-f]{12}$"))]
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub uptime_s: u64,
    pub logged_in: bool,
    pub premium: bool,
    #[schemars(length(max = 64))]
    pub quality: String,
    #[schemars(schema_with = "route_schema")]
    pub route: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Manual,
    Render,
    Netcheck,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub error: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Api,
    Stream,
    Storage,
    Pay,
    Images,
    Other,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    #[schemars(length(max = 64))]
    pub at: String,
    #[schemars(pattern("^[0-9a-f]{32}$"))]
    pub trace: String,
    #[schemars(pattern("^[0-9a-f]{16}$"))]
    pub span: String,
    pub debug: bool,
    pub service: Service,
    #[schemars(length(max = 16))]
    pub method: String,
    #[schemars(length(max = 1000))]
    pub path: String,
    #[schemars(pattern("^(direct|r[0-9]+|p[0-9]+)$"))]
    pub hop: String,
    pub status: u16,
    pub ms: u64,
    #[schemars(length(max = 1000))]
    pub error: Option<String>,
    pub bytes: u64,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Stream => "stream",
            Self::Storage => "storage",
            Self::Pay => "pay",
            Self::Images => "images",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Render,
    Unhandled,
    Rejection,
    Panic,
    Playback,
    Request,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Unhandled => "unhandled",
            Self::Rejection => "rejection",
            Self::Panic => "panic",
            Self::Playback => "playback",
            Self::Request => "request",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorEntry {
    #[schemars(length(max = 64))]
    pub at: String,
    pub kind: ErrorKind,
    #[schemars(length(max = 2000))]
    pub message: String,
    #[schemars(length(max = 8000))]
    pub stack: Option<String>,
    #[schemars(length(max = 4000))]
    pub component_stack: Option<String>,
    #[schemars(pattern("^[0-9a-f]{32}$"))]
    pub trace: Option<String>,
    #[schemars(pattern("^[0-9a-f]{16}$"))]
    pub fingerprint: String,
    pub count: u32,
    #[schemars(schema_with = "context_schema")]
    pub context: BTreeMap<String, String>,
}

impl Bundle {
    pub fn check(&self) -> Result<(), String> {
        if self.kind != BUNDLE_KIND || self.v != BUNDLE_VERSION {
            return Err("unknown bundle kind or version".to_owned());
        }
        chars("at", &self.at, SHORT)?;
        self.app.check()?;
        chars("quality", &self.session.quality, SHORT)?;
        if self.session.route.len() > 4 {
            return Err("route has too many keys".to_owned());
        }
        for (key, value) in &self.session.route {
            chars("route key", key, SHORT)?;
            chars("route value", value, SHORT)?;
        }
        if self.attempts.len() > MAX_ATTEMPTS {
            return Err("too many attempts".to_owned());
        }
        for attempt in &self.attempts {
            attempt.check()?;
        }
        if self.errors.len() > MAX_ERRORS {
            return Err("too many errors".to_owned());
        }
        for entry in &self.errors {
            entry.check()?;
        }
        if let Some(index) = self.trigger.error
            && index as usize >= self.errors.len()
        {
            return Err("trigger.error points past errors".to_owned());
        }
        if let Some(netcheck) = &self.netcheck {
            chars("netcheck", netcheck, NETCHECK)?;
        }
        if self.log_tail.len() > MAX_LOG_LINES {
            return Err("log_tail is too long".to_owned());
        }
        for line in &self.log_tail {
            chars("log_tail line", line, LOG_LINE)?;
        }
        Ok(())
    }
}

impl EventsBody {
    pub fn check(&self) -> Result<(), String> {
        self.app.check()?;
        if self.events.is_empty() || self.events.len() > MAX_EVENTS {
            return Err("events must hold 1 to 50 entries".to_owned());
        }
        for entry in &self.events {
            entry.check()?;
        }
        Ok(())
    }
}

impl App {
    pub fn check(&self) -> Result<(), String> {
        for (name, value) in [
            ("version", &self.version),
            ("os", &self.os),
            ("os_version", &self.os_version),
            ("arch", &self.arch),
            ("install", &self.install),
            ("locale", &self.locale),
        ] {
            chars(name, value, SHORT)?;
        }
        hex("app.id", &self.id, INSTALL_PATTERN)
    }
}

impl Attempt {
    fn check(&self) -> Result<(), String> {
        chars("attempt.at", &self.at, SHORT)?;
        hex("attempt.trace", &self.trace, TRACE_PATTERN)?;
        hex("attempt.span", &self.span, SPAN_PATTERN)?;
        chars("attempt.method", &self.method, 16)?;
        chars("attempt.path", &self.path, PATH)?;
        chars("attempt.hop", &self.hop, SHORT)?;
        if let Some(error) = &self.error {
            chars("attempt.error", error, ERROR)?;
        }
        Ok(())
    }
}

impl ErrorEntry {
    pub fn check(&self) -> Result<(), String> {
        chars("error.at", &self.at, SHORT)?;
        chars("error.message", &self.message, MESSAGE)?;
        if let Some(stack) = &self.stack {
            chars("error.stack", stack, STACK)?;
        }
        if let Some(stack) = &self.component_stack {
            chars("error.component_stack", stack, COMPONENT_STACK)?;
        }
        if let Some(trace) = &self.trace {
            hex("error.trace", trace, TRACE_PATTERN)?;
        }
        hex("error.fingerprint", &self.fingerprint, FINGERPRINT_PATTERN)?;
        if self.context.len() > MAX_CONTEXT_KEYS {
            return Err("error.context has too many keys".to_owned());
        }
        for (key, value) in &self.context {
            chars("error.context key", key, SHORT)?;
            chars("error.context value", value, CONTEXT_VALUE)?;
        }
        Ok(())
    }
}

fn route_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "maxProperties": 4,
        "propertyNames": { "maxLength": 64 },
        "additionalProperties": { "type": "string", "maxLength": 64 }
    })
}

fn context_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "maxProperties": 10,
        "propertyNames": { "maxLength": 64 },
        "additionalProperties": { "type": "string", "maxLength": 300 }
    })
}

fn chars(name: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().count() > max {
        return Err(format!("{name} is longer than {max} characters"));
    }
    Ok(())
}

fn hex(name: &str, value: &str, pattern: &str) -> Result<(), String> {
    let length = match pattern {
        TRACE_PATTERN => 32,
        INSTALL_PATTERN => 12,
        _ => 16,
    };
    let lower_hex = value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if value.len() != length || !lower_hex {
        return Err(format!("{name} must be {length} lowercase hex characters"));
    }
    Ok(())
}

pub fn render_diag_contract() -> String {
    let mut rendered =
        serde_json::to_string_pretty(&diag_contract()).expect("the diag contract serializes");
    rendered.push('\n');
    rendered
}

pub fn diag_contract() -> Value {
    json!({
        "version": CONTRACT_VERSION,
        "max_bundle_bytes": MAX_BUNDLE_BYTES,
        "bundle": root_schema::<Bundle>("scd-diagnostics bundle"),
        "events": root_schema::<EventsBody>("scd-diagnostics events"),
        "report": root_schema::<crate::diag_report::Report>("scd-diagnostics report"),
    })
}

fn root_schema<T: JsonSchema>(title: &str) -> Value {
    let mut schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    schema.insert("title".to_owned(), Value::from(title));
    Value::from(schema)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn sample() -> Bundle {
        serde_json::from_value(json!({
            "kind": "scd-diagnostics",
            "v": 1,
            "at": "2026-10-29T10:00:00.000Z",
            "app": { "version": "8.4.25", "os": "windows", "os_version": "10.0.22631", "arch": "x86_64", "install": "nsis", "locale": "ru", "id": "a1b2c3d4e5f6" },
            "session": { "uptime_s": 3600, "logged_in": true, "premium": false, "quality": "hq", "route": { "main": "relay:r2", "star": "direct" } },
            "trigger": { "kind": "manual", "error": null },
            "attempts": [{ "at": "2026-10-29T10:00:00.000Z", "trace": "4bf92f3577b3da6a3ce929d0e0e4736f", "span": "00f067aa0ba902b7", "debug": true, "service": "stream", "method": "GET", "path": "/stream/soundcloud:tracks:123?quality=hq", "hop": "r2", "status": 0, "ms": 10012, "error": "connect timeout after 10s", "bytes": 0 }],
            "errors": [{ "at": "2026-10-29T10:00:00.000Z", "kind": "render", "message": "boom", "stack": null, "component_stack": null, "trace": null, "fingerprint": "0123456789abcdef", "count": 1, "context": { "route": "/library" } }],
            "netcheck": null,
            "log_tail": ["a line"]
        }))
        .expect("the sample parses")
    }

    #[test]
    fn the_exported_file_is_what_the_crate_generates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CONTRACT_PATH);
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();

        assert!(
            on_disk == render_diag_contract(),
            "{} drifted from backend-contracts; regenerate it with `cargo run --bin export-diag-contract`",
            path.display()
        );
    }

    #[test]
    fn the_sample_bundle_passes_the_caps() {
        assert_eq!(sample().check(), Ok(()));
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let mut value = serde_json::to_value(sample()).expect("serializes");
        value["extra"] = json!(1);

        assert!(serde_json::from_value::<Bundle>(value).is_err());
    }

    #[test]
    fn a_long_message_is_refused() {
        let mut bundle = sample();
        bundle.errors[0].message = "x".repeat(2001);

        assert!(bundle.check().is_err());
    }

    #[test]
    fn a_non_hex_trace_is_refused() {
        let mut bundle = sample();
        bundle.attempts[0].trace = "4bf92f3577b3da6a:3ce929d0e0e4736f".to_owned();

        assert!(bundle.check().is_err());
    }

    #[test]
    fn too_many_context_keys_are_refused() {
        let mut bundle = sample();
        for index in 0..11 {
            bundle.errors[0]
                .context
                .insert(format!("k{index}"), "v".to_owned());
        }

        assert!(bundle.check().is_err());
    }
}
