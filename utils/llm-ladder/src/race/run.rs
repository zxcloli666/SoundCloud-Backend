use std::time::Duration;

use tokio::time::Instant;

use crate::breaker::Pass;
use crate::document::Document;
use crate::provider::StepStream;
use crate::request::Usage;

const USAGE_WAIT: Duration = Duration::from_secs(1);

pub(super) struct Run {
    pub step: usize,
    pub stream: StepStream,
    pub pass: Pass,
    pub document: Document,
    pub held: String,
    pub usage: Usage,
    pub received: usize,
    pub heard: bool,
    pub started: Instant,
    pub expires: Instant,
    pub first_output: Option<Duration>,
    answered_at: Option<Instant>,
}

impl Run {
    pub fn new(
        step: usize,
        stream: StepStream,
        pass: Pass,
        started: Instant,
        expires: Instant,
    ) -> Self {
        Self {
            step,
            stream,
            pass,
            document: Document::default(),
            held: String::new(),
            usage: Usage::default(),
            received: 0,
            heard: false,
            started,
            expires,
            first_output: None,
            answered_at: None,
        }
    }

    pub fn read(&mut self, text: &str) {
        self.heard = true;
        self.received += text.len();
        let visible = self.document.push(text);
        if !visible.is_empty() {
            let started = self.started;
            self.first_output.get_or_insert_with(|| started.elapsed());
            self.held.push_str(&visible);
        }
        if self.document.is_complete() {
            self.answered_at.get_or_insert_with(Instant::now);
        }
    }

    pub fn count(&mut self, usage: Usage) {
        self.heard = true;
        self.usage = usage;
    }

    pub fn wake_at(&self, deadline: Instant) -> Instant {
        let tail = self.answered_at.map_or(deadline, |at| at + USAGE_WAIT);
        self.expires.min(deadline).min(tail)
    }
}
