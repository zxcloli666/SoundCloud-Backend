use std::collections::VecDeque;

use bytes::Bytes;
use futures::stream::{self, BoxStream, Stream, StreamExt};

use super::StepEvent;
use super::http::{MAX_ANSWER_BYTES, cause};
use crate::observer::Failure;

const DATA: &str = "data:";

pub(crate) trait Reader: Send + 'static {
    fn read(&mut self, data: &str) -> Read;

    fn stop(&self) -> Option<Failure>;
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
            Some(Ok(chunk)) => match self.lines.push(&chunk) {
                Some(lines) => {
                    for data in lines {
                        self.take(&data);
                    }
                }
                None => {
                    tracing::warn!(provider = %self.provider, limit = MAX_ANSWER_BYTES, "llm provider stream line is too long");
                    self.ready.push_back(Err(Failure::Transport));
                }
            },
            Some(Err(error)) => {
                tracing::warn!(provider = %self.provider, cause = cause(&error), "llm provider stream broke");
                self.ready.push_back(Err(Failure::Transport));
            }
            None => {
                if let Some(data) = self.lines.finish() {
                    self.take(&data);
                }
                self.end();
            }
        }
    }

    fn take(&mut self, data: &str) {
        if self.over {
            return;
        }
        match self.reader.read(data) {
            Read::Events(events) => self.ready.extend(events.into_iter().map(Ok)),
            Read::End => self.end(),
            Read::Failed(failure) => self.fail(failure),
        }
    }

    fn end(&mut self) {
        if self.over {
            return;
        }
        match self.reader.stop() {
            Some(failure) => self.fail(failure),
            None => self.over = true,
        }
    }

    fn fail(&mut self, failure: Failure) {
        tracing::warn!(provider = %self.provider, ?failure, "llm provider stream reported a failure");
        self.ready.push_back(Err(failure));
    }
}

#[derive(Default)]
pub(crate) struct Lines {
    pending: Vec<u8>,
}

impl Lines {
    pub fn push(&mut self, chunk: &[u8]) -> Option<Vec<String>> {
        self.pending.extend_from_slice(chunk);
        let mut payloads = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            payloads.extend(payload(&line));
        }
        (self.pending.len() <= MAX_ANSWER_BYTES).then_some(payloads)
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
        let mut payloads = whole.push(wire.as_bytes()).unwrap();
        payloads.extend(whole.finish());
        assert_eq!(payloads, ["{\"a\":\"привет\"}", "{\"b\":2}", "[DONE]"]);

        let mut cut = Lines::default();
        let mut payloads = Vec::new();
        for byte in wire.as_bytes() {
            payloads.extend(cut.push(&[*byte]).unwrap());
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
                .unwrap()
                .is_empty()
        );
        assert_eq!(lines.finish(), None);
    }

    #[test]
    fn a_line_that_never_ends_is_cut_off_at_the_answer_limit() {
        let mut lines = Lines::default();
        let piece = vec![b'x'; MAX_ANSWER_BYTES / 4];
        for _ in 0..4 {
            assert_eq!(lines.push(&piece), Some(Vec::new()));
        }
        assert_eq!(lines.push(b"x"), None);
        let mut ended = Lines::default();
        assert_eq!(ended.push(&piece).map(|lines| lines.len()), Some(0));
        assert_eq!(ended.push(b"\ndata: {}\n"), Some(vec!["{}".to_owned()]));
    }
}
