use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use super::service::{Hold, Hosting, Live, VoiceSpeak};
use super::stores::{Ask, Lane, Objects, Replies, StoreError};

pub const CLIP: &[u8] = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00OpusHead";
pub const QUICK: Hold = Hold {
    length: Duration::from_millis(300),
    every: Duration::from_millis(5),
};

#[derive(Default)]
pub struct Shelf {
    objects: Mutex<HashMap<String, Bytes>>,
    broken: AtomicBool,
    reads: AtomicUsize,
}

#[derive(Default)]
pub struct Board {
    messages: Mutex<HashMap<String, Bytes>>,
    broken: AtomicBool,
}

#[derive(Clone, Copy)]
pub enum Answer {
    Clip,
    Encoded(&'static str),
    Refuse(&'static str),
    Malformed,
    Silent,
}

pub struct Queue {
    answer: Answer,
    replies: Arc<Board>,
    asked: Mutex<Vec<Ask>>,
    depth: AtomicU64,
    refusing: AtomicBool,
}

pub struct Speaking {
    pub speak: Arc<VoiceSpeak>,
    pub lane: Arc<Queue>,
    pub replies: Arc<Board>,
    pub phrases: Arc<Shelf>,
}

pub fn speaking(answer: Answer, lines: usize) -> Speaking {
    hosted(answer, lines, true)
}

pub fn uncached(answer: Answer, lines: usize) -> Speaking {
    hosted(answer, lines, false)
}

fn hosted(answer: Answer, lines: usize, cached: bool) -> Speaking {
    let replies = Arc::new(Board::default());
    let phrases = Arc::new(Shelf::default());
    let lane = Arc::new(Queue {
        answer,
        replies: replies.clone(),
        asked: Mutex::default(),
        depth: AtomicU64::new(0),
        refusing: AtomicBool::new(false),
    });
    let hosting = Hosting {
        lane: lane.clone(),
        replies: replies.clone(),
        phrases: cached.then(|| phrases.clone() as Arc<dyn Objects>),
    };
    Speaking {
        speak: VoiceSpeak::hosted(lines, hosting, QUICK),
        lane,
        replies,
        phrases,
    }
}

pub fn reply(answer: Answer) -> Option<Value> {
    let data = |clip: Value| json!({"ok": true, "data": {"seconds": 1.5, "clip": clip}});
    match answer {
        Answer::Clip => Some(data(Value::from(STANDARD.encode(CLIP)))),
        Answer::Encoded(clip) => Some(data(Value::from(clip))),
        Answer::Refuse(error) => Some(json!({"ok": false, "error": error})),
        Answer::Malformed => Some(json!({"ok": true, "data": {"length": 1}})),
        Answer::Silent => None,
    }
}

impl Speaking {
    pub fn live(&self) -> &Live {
        self.speak.live().expect("a live speak service")
    }
}

impl Shelf {
    pub fn put(&self, name: &str, bytes: &[u8]) {
        self.objects
            .lock()
            .unwrap()
            .insert(name.to_owned(), Bytes::copy_from_slice(bytes));
    }

    pub fn get(&self, name: &str) -> Option<Bytes> {
        self.objects.lock().unwrap().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.objects.lock().unwrap().keys().cloned().collect()
    }

    pub fn break_down(&self) {
        self.broken.store(true, Ordering::Relaxed);
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
}

impl Objects for Shelf {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, StoreError>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::Relaxed);
            working(&self.broken).map(|()| self.get(name))
        })
    }

    fn write<'a>(&'a self, name: &'a str, bytes: Bytes) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            working(&self.broken)?;
            self.put(name, &bytes);
            Ok(())
        })
    }
}

impl Board {
    pub fn put(&self, subject: &str, payload: &[u8]) {
        self.messages
            .lock()
            .unwrap()
            .insert(subject.to_owned(), Bytes::copy_from_slice(payload));
    }

    pub fn get(&self, subject: &str) -> Option<Bytes> {
        self.messages.lock().unwrap().get(subject).cloned()
    }

    pub fn subjects(&self) -> Vec<String> {
        let mut subjects: Vec<String> = self.messages.lock().unwrap().keys().cloned().collect();
        subjects.sort();
        subjects
    }

    pub fn break_down(&self) {
        self.broken.store(true, Ordering::Relaxed);
    }
}

impl Replies for Board {
    fn last<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, StoreError>> {
        Box::pin(async move { working(&self.broken).map(|()| self.get(subject)) })
    }

    fn put<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            working(&self.broken)?;
            Board::put(self, subject, &payload);
            Ok(())
        })
    }
}

impl Queue {
    pub fn asked(&self) -> Vec<Ask> {
        self.asked.lock().unwrap().clone()
    }

    pub fn fill(&self, depth: u64) {
        self.depth.store(depth, Ordering::Relaxed);
    }

    pub fn refuse(&self) {
        self.refusing.store(true, Ordering::Relaxed);
    }
}

impl Lane for Queue {
    fn depth(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move { Ok(self.depth.load(Ordering::Relaxed)) })
    }

    fn ask(&self, ask: Ask) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            working(&self.refusing)?;
            if let Some(reply) = reply(self.answer) {
                Board::put(&self.replies, &ask.reply_to, reply.to_string().as_bytes());
            }
            self.asked.lock().unwrap().push(ask);
            Ok(())
        })
    }
}

fn working(broken: &AtomicBool) -> Result<(), StoreError> {
    if broken.load(Ordering::Relaxed) {
        Err(StoreError::Unavailable)
    } else {
        Ok(())
    }
}
