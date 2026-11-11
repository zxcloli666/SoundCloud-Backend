use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream::{self, StreamExt};
use llm_ladder::{AnswerStream, Commit, LadderError};
use serde::Serialize;
use serde_json::json;

use super::answer::{Answer, Piece};
use super::tools::Tools;

const KEEPALIVE: Duration = Duration::from_secs(15);

pub fn commit(tools: Arc<Tools>) -> Commit {
    Arc::new(move |held: &str| {
        Answer::default()
            .push(held)
            .into_iter()
            .any(|piece| match piece {
                Piece::Say(text) => !text.trim().is_empty(),
                Piece::Action(raw) => tools.fit(raw).is_some(),
            })
    })
}

pub fn respond(answer: AnswerStream, tools: Arc<Tools>, started: Instant) -> Response {
    let reply = Reply {
        answer,
        parser: Answer::default(),
        tools,
        ready: VecDeque::new(),
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

struct Reply {
    answer: AnswerStream,
    parser: Answer,
    tools: Arc<Tools>,
    ready: VecDeque<Event>,
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
            Some(Ok(llm_ladder::Event::Done(_))) => self.close("done", "done", json!({})),
            Some(Err(LadderError::Deadline)) => self.fail("deadline"),
            Some(Err(_)) | None => self.fail("provider_failed"),
        }
    }

    fn take(&mut self, delta: &str) {
        for piece in self.parser.push(delta) {
            match piece {
                Piece::Say(text) => self.send("say", json!({"text": text})),
                Piece::Action(raw) => match self.tools.fit(raw) {
                    Some(action) => {
                        self.tally.actions += 1;
                        self.send("action", action);
                    }
                    None => self.tally.dropped += 1,
                },
            }
        }
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
