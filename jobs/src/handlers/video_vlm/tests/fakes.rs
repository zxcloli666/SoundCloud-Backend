use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use backend_contracts::pipeline::VideoLabel;
use serde_json::Value;

use super::super::haiku::{Judged, Vision};
use crate::handlers::video_accounts::Notify;
use crate::handlers::video_gen::Lanes;
use crate::llm::anthropic::{Answer, LlmError, Spend};

pub(super) const HAIKU: &str = "example-haiku";
pub(super) const CALL_USD: f64 = 0.0004;

#[derive(Clone, Default)]
pub(super) struct Wire {
    pub(super) sent: Arc<Mutex<Vec<(String, Value, String)>>>,
    pub(super) down: Arc<AtomicBool>,
}

impl Wire {
    pub(super) fn requests(&self) -> Vec<(Value, String)> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(_, payload, id)| (payload.clone(), id.clone()))
            .collect()
    }
}

impl Lanes for Wire {
    async fn publish(
        &self,
        subject: &'static str,
        payload: Value,
        message_id: String,
    ) -> anyhow::Result<()> {
        if self.down.load(Ordering::SeqCst) {
            anyhow::bail!("nats is down");
        }
        self.sent
            .lock()
            .unwrap()
            .push((subject.to_owned(), payload, message_id));
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct Eyes {
    answers: Arc<Mutex<VecDeque<Result<VideoLabel, LlmError>>>>,
    pub(super) seen: Arc<Mutex<Vec<Vec<Vec<u8>>>>>,
}

impl Eyes {
    pub(super) fn answers(&self, answer: Result<VideoLabel, LlmError>) {
        self.answers.lock().unwrap().push_back(answer);
    }
}

impl Vision for Eyes {
    async fn judge(&self, frames: &[Vec<u8>]) -> Answer<Judged> {
        self.seen.lock().unwrap().push(frames.to_vec());
        let next = self.answers.lock().unwrap().pop_front();
        let result = next
            .unwrap_or(Err(LlmError::Invalid("unscripted".to_owned())))
            .map(|label| Judged {
                label,
                model: HAIKU.to_owned(),
            });
        let spend = match &result {
            Err(LlmError::Down(_)) => Vec::new(),
            _ => vec![Spend {
                call: "vlm",
                provider: "haiku".to_owned(),
                model: HAIKU.to_owned(),
                input_tokens: 2_300,
                output_tokens: 120,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                usd: CALL_USD,
            }],
        };
        Answer { result, spend }
    }
}

#[derive(Clone, Default)]
pub(super) struct Inbox {
    pub(super) messages: Arc<Mutex<Vec<String>>>,
}

impl Inbox {
    pub(super) fn count(&self) -> usize {
        self.messages.lock().unwrap().len()
    }
}

impl Notify for Inbox {
    async fn send(&self, text: &str) -> Result<(), String> {
        self.messages.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}
