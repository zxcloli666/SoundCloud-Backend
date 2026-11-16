use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use llm_ladder::{Hosting, Ladder, Ladders, Settings, WorkerError, WorkerTransport};
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use super::metrics::LadderMetrics;

pub const FIRST: &str = "test.first";
pub const SECOND: &str = "test.second";

const LADDER: &str = r#"
[providers.first]
kind = "worker"
subject = "test.first"
max_output_tokens = 300
total_timeout_ms = 2000

[providers.second]
kind = "worker"
subject = "test.second"
max_output_tokens = 300
total_timeout_ms = 2000

[ladders.refine]
steps = ["first", "second"]
hedge_after_ms = 1500
deadline_ms = 4000
breaker = { failures = 5, window_s = 60, open_s = 60 }
"#;

#[derive(Default)]
pub struct Scripted {
    answers: Mutex<HashMap<String, Value>>,
    asked: Mutex<Vec<(String, Value)>>,
}

impl Scripted {
    pub fn answer(&self, subject: &str, document: Value) {
        if let Ok(mut answers) = self.answers.lock() {
            answers.insert(subject.to_owned(), json!({ "text": document.to_string() }));
        }
    }

    pub fn asked(&self) -> Vec<(String, Value)> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }
}

impl WorkerTransport for Scripted {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        _timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(async move {
            let payload = serde_json::from_slice(&payload).unwrap_or(Value::Null);
            if let Ok(mut asked) = self.asked.lock() {
                asked.push((subject.to_owned(), payload));
            }
            self.answers
                .lock()
                .ok()
                .and_then(|answers| answers.get(subject).cloned())
                .ok_or(WorkerError::Rejected)
        })
    }
}

pub fn refine_ladder(worker: Arc<Scripted>) -> anyhow::Result<Arc<Ladder>> {
    let (settings, _): (Settings, IgnoredAny) = Settings::parse(LADDER)?;
    let hosting = Hosting {
        jetstream: None,
        worker: Some(worker as Arc<dyn WorkerTransport>),
        observer: Arc::new(LadderMetrics),
    };
    Ladders::build(&settings, hosting)?
        .get("refine")
        .ok_or_else(|| anyhow::anyhow!("the test ladder has no refine ladder"))
}

pub fn answer_fields(schema: &Value) -> Vec<String> {
    let mut fields = Vec::new();
    collect_fields(schema, &mut fields);
    fields
}

fn collect_fields(schema: &Value, fields: &mut Vec<String>) {
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, inner) in properties {
            fields.push(name.clone());
            collect_fields(inner, fields);
        }
    }
    let options = schema.get("anyOf").and_then(Value::as_array);
    for option in options.into_iter().flatten() {
        collect_fields(option, fields);
    }
}
