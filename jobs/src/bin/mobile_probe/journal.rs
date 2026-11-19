use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sc_mobile::{MobileError, MobileReply, MobileRequest};
use serde_json::{Value, json};
use url::Url;

use crate::scrub::Scrubber;

const TEXT_KEPT_CHARS: usize = 4000;
const HEAD_KEPT_BYTES: usize = 64;
const ERROR_KEPT_CHARS: usize = 300;

pub struct Journal {
    out: PathBuf,
    scrubber: Scrubber,
    rows: Vec<Row>,
}

pub struct Sent {
    method: String,
    url: String,
    body: Option<Value>,
}

struct Row {
    step: String,
    status: Option<u16>,
    plan: Option<String>,
    outcome: Outcome,
    notes: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Skipped,
}

impl Sent {
    pub fn of(request: &MobileRequest) -> Self {
        Self {
            method: request.method().to_string(),
            url: request.url().map(String::from).unwrap_or_default(),
            body: request.body().map(body_value),
        }
    }

    pub fn get(url: &Url) -> Self {
        Self {
            method: "GET".to_owned(),
            url: url.to_string(),
            body: None,
        }
    }

    fn fixture(self) -> Value {
        json!({ "method": self.method, "url": self.url, "body": self.body })
    }
}

impl Journal {
    pub fn open(out: &Path, scrubber: Scrubber) -> io::Result<Self> {
        fs::create_dir_all(out)?;
        Ok(Self {
            out: out.to_path_buf(),
            scrubber,
            rows: Vec::new(),
        })
    }

    pub fn hide(&mut self, value: &str) {
        self.scrubber.hide(value);
    }

    pub fn reply(
        &mut self,
        step: &str,
        sent: Sent,
        outcome: &Result<MobileReply, MobileError>,
        notes: Vec<String>,
    ) {
        let (response, status, plan, outcome) = match outcome {
            Ok(reply) => {
                let body = body_value(&reply.body);
                let passed = reply.is_success() && graphql_errors(&body).is_none();
                let plan = reply.plan.as_ref().map(|plan| plan.as_str().to_owned());
                let response = json!({
                    "status": reply.status,
                    "plan": plan,
                    "location": reply.location.as_ref().map(Url::as_str),
                    "body": body,
                });
                (response, Some(reply.status), plan, passed_or_failed(passed))
            }
            Err(error) => (
                json!({ "error": error.to_string() }),
                None,
                None,
                Outcome::Failed,
            ),
        };
        let fixture = json!({ "step": step, "request": sent.fixture(), "response": response });
        self.push(
            Row {
                step: step.to_owned(),
                status,
                plan,
                outcome,
                notes,
            },
            fixture,
        );
    }

    pub fn event(&mut self, step: &str, passed: bool, details: Value, notes: Vec<String>) {
        let fixture = json!({ "step": step, "details": details });
        self.push(
            Row {
                step: step.to_owned(),
                status: None,
                plan: None,
                outcome: passed_or_failed(passed),
                notes,
            },
            fixture,
        );
    }

    pub fn skip(&mut self, step: &str, why: &str) {
        self.rows.push(Row {
            step: step.to_owned(),
            status: None,
            plan: None,
            outcome: Outcome::Skipped,
            notes: vec![why.to_owned()],
        });
    }

    pub fn failed(&self) -> bool {
        self.rows.iter().any(|row| row.outcome == Outcome::Failed)
    }

    pub fn print_summary(&self) {
        println!(
            "{:<3} {:<26} {:<6} {:<12} {:<6} notes",
            "#", "step", "status", "plan", "result"
        );
        for (index, row) in self.rows.iter().enumerate() {
            let status = row
                .status
                .map_or("-".to_owned(), |status| status.to_string());
            let result = match row.outcome {
                Outcome::Passed => "ok",
                Outcome::Failed => "FAIL",
                Outcome::Skipped => "skip",
            };
            println!(
                "{:<3} {:<26} {:<6} {:<12} {:<6} {}",
                index + 1,
                row.step,
                status,
                row.plan.as_deref().unwrap_or("-"),
                result,
                self.scrubber.text(&row.notes.join("; "))
            );
        }
    }

    fn push(&mut self, row: Row, fixture: Value) {
        let path = self.out.join(format!(
            "{:02}-{}.json",
            self.rows.len() + 1,
            file_name(&row.step)
        ));
        let scrubbed = self.scrubber.value(fixture);
        let written = serde_json::to_vec_pretty(&scrubbed)
            .map_err(io::Error::other)
            .and_then(|bytes| fs::write(&path, bytes));
        if let Err(error) = written {
            eprintln!("could not write the fixture {}: {error}", path.display());
        }
        self.rows.push(row);
    }
}

pub fn json_of(outcome: &Result<MobileReply, MobileError>) -> Option<Value> {
    let reply = outcome.as_ref().ok()?;
    serde_json::from_slice(&reply.body).ok()
}

pub fn graphql_errors(body: &Value) -> Option<String> {
    let errors = body
        .get("errors")?
        .as_array()
        .filter(|errors| !errors.is_empty())?;
    let messages: Vec<&str> = errors
        .iter()
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error without a message")
        })
        .collect();
    Some(
        format!("graphql errors: {}", messages.join(" | "))
            .chars()
            .take(ERROR_KEPT_CHARS)
            .collect(),
    )
}

pub fn failure_note(outcome: &Result<MobileReply, MobileError>) -> Option<String> {
    match outcome {
        Err(error) => Some(error.to_string()),
        Ok(reply) => json_of(outcome)
            .as_ref()
            .and_then(graphql_errors)
            .or_else(|| {
                (!reply.is_success()).then(|| {
                    let text = String::from_utf8_lossy(&reply.body);
                    format!(
                        "answer: {}",
                        text.chars().take(ERROR_KEPT_CHARS).collect::<String>()
                    )
                })
            }),
    }
}

fn passed_or_failed(passed: bool) -> Outcome {
    if passed {
        Outcome::Passed
    } else {
        Outcome::Failed
    }
}

fn body_value(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return value;
    }
    match std::str::from_utf8(body) {
        Ok(text) => json!({ "text": text.chars().take(TEXT_KEPT_CHARS).collect::<String>() }),
        Err(_) => json!({
            "bytes": body.len(),
            "head_hex": body
                .iter()
                .take(HEAD_KEPT_BYTES)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        }),
    }
}

fn file_name(step: &str) -> String {
    step.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
