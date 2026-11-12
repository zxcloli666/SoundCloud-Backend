use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream::{self, StreamExt};
use llm_ladder::{AnswerStream, Commit, Form, LadderError};
use serde::Serialize;
use serde_json::json;

use super::answer::{Answer, Piece};
use super::tools::{Action, Tools};

const KEEPALIVE: Duration = Duration::from_secs(15);

pub struct Offered {
    tools: Tools,
    quiet: bool,
    answered_plainly: AtomicBool,
}

pub fn commit(offered: Arc<Offered>) -> Commit {
    Arc::new(move |form, held| offered.accepts(form, held))
}

pub fn respond(answer: AnswerStream, offered: Arc<Offered>, started: Instant) -> Response {
    let reply = Reply {
        answer,
        parser: Answer::default(),
        offered,
        ready: VecDeque::new(),
        spoke: false,
        tally: Tally {
            started,
            first_event: None,
            actions: 0,
            dropped: 0,
            outcome: None,
        },
    };
    let events = stream::unfold(reply, |mut reply| async move {
        loop {
            if let Some(event) = reply.ready.pop_front() {
                return Some((Ok::<_, Infallible>(event), reply));
            }
            if reply.tally.outcome.is_some() {
                return None;
            }
            reply.pull().await;
        }
    });
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(KEEPALIVE))
        .into_response()
}

impl Offered {
    pub fn new(tools: Tools, quiet: bool) -> Arc<Self> {
        Arc::new(Self {
            tools,
            quiet,
            answered_plainly: AtomicBool::new(false),
        })
    }

    fn accepts(&self, form: Form, held: &str) -> bool {
        let mut answer = Answer::default();
        let pieces = answer.push(held);
        let accepted = match form {
            Form::Full if self.quiet && answer.is_closed() && pieces.iter().all(blank) => true,
            Form::Full => pieces.into_iter().any(|piece| match piece {
                Piece::Say(text) => !text.trim().is_empty(),
                Piece::Action(raw) => self.tools.fit(raw).is_some(),
            }),
            Form::Plain => only_speaks(pieces),
        };
        if accepted {
            self.answered_plainly
                .store(form == Form::Plain, Ordering::Relaxed);
        }
        accepted
    }

    fn fit(&self, raw: serde_json::Value) -> Option<Action> {
        match self.answered_plainly.load(Ordering::Relaxed) {
            true => None,
            false => self.tools.fit(raw),
        }
    }
}

fn blank(piece: &Piece) -> bool {
    matches!(piece, Piece::Say(text) if text.trim().is_empty())
}

fn only_speaks(pieces: Vec<Piece>) -> bool {
    let mut speaks = false;
    for piece in pieces {
        match piece {
            Piece::Action(_) => return false,
            Piece::Say(text) => speaks |= !text.trim().is_empty(),
        }
    }
    speaks
}

struct Reply {
    answer: AnswerStream,
    parser: Answer,
    offered: Arc<Offered>,
    ready: VecDeque<Event>,
    spoke: bool,
    tally: Tally,
}

struct Tally {
    started: Instant,
    first_event: Option<Duration>,
    actions: u64,
    dropped: u64,
    outcome: Option<&'static str>,
}

impl Reply {
    async fn pull(&mut self) {
        match self.answer.next().await {
            Some(Ok(llm_ladder::Event::Delta(delta))) => self.take(&delta),
            Some(Ok(llm_ladder::Event::Done(_))) => self.finish(),
            Some(Err(LadderError::Deadline)) => self.fail("deadline"),
            Some(Err(_)) | None => self.fail("provider_failed"),
        }
    }

    fn take(&mut self, delta: &str) {
        for piece in self.parser.push(delta) {
            match piece {
                Piece::Say(text) if !self.spoke && text.trim().is_empty() => {}
                Piece::Say(text) => {
                    self.spoke = true;
                    self.send("say", json!({"text": text}));
                }
                Piece::Action(raw) => match self.offered.fit(raw) {
                    Some(action) => {
                        self.tally.actions += 1;
                        self.send("action", action);
                    }
                    None => self.tally.dropped += 1,
                },
            }
        }
    }

    fn finish(&mut self) {
        let silent = self.offered.quiet && !self.spoke && self.tally.actions == 0;
        if silent {
            self.send("quiet", json!({}));
        }
        let outcome = match silent {
            true => "quiet",
            false => "done",
        };
        self.close(outcome, "done", json!({}));
    }

    fn fail(&mut self, code: &'static str) {
        self.close(code, "error", json!({"code": code}));
    }

    fn close(&mut self, outcome: &'static str, event: &'static str, data: impl Serialize) {
        self.send(event, data);
        self.tally.outcome = Some(outcome);
    }

    fn send(&mut self, event: &'static str, data: impl Serialize) {
        let data = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_owned());
        self.tally
            .first_event
            .get_or_insert_with(|| self.tally.started.elapsed());
        self.ready
            .push_back(Event::default().event(event).data(data));
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        crate::metrics::record_voice_chat(
            self.outcome.unwrap_or("cancelled"),
            self.started.elapsed(),
            self.first_event,
            self.actions,
            self.dropped,
        );
    }
}

#[cfg(test)]
#[path = "reply_tests.rs"]
mod tests;
