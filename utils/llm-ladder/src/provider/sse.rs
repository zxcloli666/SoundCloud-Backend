use std::collections::VecDeque;

use bytes::Bytes;
use futures::stream::{self, BoxStream, Stream, StreamExt};

use super::StepEvent;
use super::http::cause;
use crate::observer::Failure;

const DATA: &str = "data:";

pub(crate) trait Reader: Send + 'static {
    fn read(&mut self, data: &str) -> Read;
}

pub(crate) enum Read {
    Events(Vec<StepEvent>),
    End,
    Failed(Failure),
}

pub(crate) fn events<R: Reader>(
    provider: String,
    response: wreq::Response,
    reader: R,
) -> impl Stream<Item = Result<StepEvent, Failure>> + Send + 'static {
    let feed = Feed {
        provider,
        body: response.bytes_stream().boxed(),
        lines: Lines::default(),
        reader,
        ready: VecDeque::new(),
        over: false,
    };
    stream::unfold(feed, |mut feed| async move {
        loop {
            if let Some(item) = feed.ready.pop_front() {
                if item.is_err() {
                    feed.ready.clear();
                    feed.over = true;
                }
                return Some((item, feed));
            }
            if feed.over {
                return None;
            }
            feed.pull().await;
        }
    })
}

struct Feed<R> {
    provider: String,
    body: BoxStream<'static, Result<Bytes, wreq::Error>>,
    lines: Lines,
    reader: R,
    ready: VecDeque<Result<StepEvent, Failure>>,
    over: bool,
}

impl<R: Reader> Feed<R> {
    async fn pull(&mut self) {
        match self.body.next().await {
            Some(Ok(chunk)) => {
                for data in self.lines.push(&chunk) {
                    self.take(&data);
                }
            }
            Some(Err(error)) => {
                tracing::warn!(provider = %self.provider, cause = cause(&error), "llm provider stream broke");
                self.ready.push_back(Err(Failure::Transport));
            }
            None => {
                if let Some(data) = self.lines.finish() {
                    self.take(&data);
                }
                self.over = true;
            }
        }
    }

    fn take(&mut self, data: &str) {
        if self.over {
            return;
        }
        match self.reader.read(data) {
            Read::Events(events) => self.ready.extend(events.into_iter().map(Ok)),
            Read::End => self.over = true,
            Read::Failed(failure) => {
                tracing::warn!(provider = %self.provider, ?failure, "llm provider stream reported a failure");
                self.ready.push_back(Err(failure));
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct Lines {
    pending: Vec<u8>,
}

impl Lines {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut payloads = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            payloads.extend(payload(&line));
        }
        payloads
    }

    pub fn finish(&mut self) -> Option<String> {
        payload(&std::mem::take(&mut self.pending))
    }
}

fn payload(line: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(line)
        .ok()?
        .trim_end_matches(['\n', '\r']);
    let data = line.strip_prefix(DATA)?;
    let data = data.strip_prefix(' ').unwrap_or(data);
    (!data.is_empty()).then(|| data.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_lines_come_out_whole_however_the_bytes_are_cut() {
        let wire = "event: message\ndata: {\"a\":\"привет\"}\n\n: keepalive\ndata:{\"b\":2}\r\n\r\ndata: [DONE]";
        let mut whole = Lines::default();
        let mut payloads = whole.push(wire.as_bytes());
        payloads.extend(whole.finish());
        assert_eq!(payloads, ["{\"a\":\"привет\"}", "{\"b\":2}", "[DONE]"]);

        let mut cut = Lines::default();
        let mut payloads = Vec::new();
        for byte in wire.as_bytes() {
            payloads.extend(cut.push(&[*byte]));
        }
        payloads.extend(cut.finish());
        assert_eq!(payloads, ["{\"a\":\"привет\"}", "{\"b\":2}", "[DONE]"]);
    }

    #[test]
    fn lines_that_carry_no_data_are_skipped() {
        let mut lines = Lines::default();
        assert!(
            lines
                .push(b"event: ping\nid: 7\nretry: 100\n\ndata:\n")
                .is_empty()
        );
        assert_eq!(lines.finish(), None);
    }
}
