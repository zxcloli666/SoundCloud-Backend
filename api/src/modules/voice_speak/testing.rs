use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::future::BoxFuture;
use llm_ladder::{WorkerError, WorkerTransport};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::objects::{ObjectError, Objects};
use super::service::{Hold, Hosting, Live, VoiceSpeak};

pub const CLIP: &[u8] = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00OpusHead";
pub const QUICK: Hold = Hold {
    length: Duration::from_millis(300),
    every: Duration::from_millis(5),
};
const SETTLE_BUDGET: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Shelf {
    objects: Mutex<HashMap<String, Bytes>>,
    broken: AtomicBool,
    reads: AtomicUsize,
}

#[derive(Clone, Copy)]
pub enum Answer {
    Clip,
    Encoded(&'static str),
    Refuse(WorkerError),
    Malformed,
}

pub struct Voice {
    answer: Answer,
    asked: Mutex<Vec<Asked>>,
    gate: Semaphore,
}

#[derive(Clone, Debug)]
pub struct Asked {
    pub subject: String,
    pub request: Value,
    pub timeout: Duration,
}

pub struct Speaking {
    pub speak: Arc<VoiceSpeak>,
    pub voice: Arc<Voice>,
    pub clips: Arc<Shelf>,
    pub phrases: Arc<Shelf>,
    lines: usize,
}

pub fn speaking(answer: Answer, lines: usize) -> Speaking {
    hosted(answer, lines, Semaphore::MAX_PERMITS)
}

pub fn held(answer: Answer, lines: usize) -> Speaking {
    hosted(answer, lines, 0)
}

fn hosted(answer: Answer, lines: usize, passes: usize) -> Speaking {
    let clips = Arc::new(Shelf::default());
    let phrases = Arc::new(Shelf::default());
    let voice = Arc::new(Voice {
        answer,
        asked: Mutex::default(),
        gate: Semaphore::new(passes),
    });
    let hosting = Hosting {
        worker: voice.clone(),
        clips: clips.clone(),
        phrases: phrases.clone(),
    };
    Speaking {
        speak: VoiceSpeak::hosted(lines, hosting, QUICK),
        voice,
        clips,
        phrases,
        lines,
    }
}

impl Speaking {
    pub fn live(&self) -> &Arc<Live> {
        self.speak.live().expect("a live speak service")
    }

    pub async fn settled(&self) {
        let live = self.live();
        tokio::time::timeout(SETTLE_BUDGET, async {
            while live.lines.available_permits() < self.lines {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("every line settles");
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

    fn working(&self) -> Result<(), ObjectError> {
        if self.broken.load(Ordering::Relaxed) {
            Err(ObjectError::Unavailable)
        } else {
            Ok(())
        }
    }
}

impl Objects for Shelf {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, ObjectError>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.working().map(|()| self.get(name))
        })
    }

    fn write<'a>(&'a self, name: &'a str, bytes: Bytes) -> BoxFuture<'a, Result<(), ObjectError>> {
        Box::pin(async move {
            self.working()?;
            self.put(name, &bytes);
            Ok(())
        })
    }
}

impl Voice {
    pub fn open(&self) {
        self.gate.add_permits(Semaphore::MAX_PERMITS);
    }

    pub fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

impl WorkerTransport for Voice {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(async move {
            let request: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
            self.asked.lock().unwrap().push(Asked {
                subject: subject.to_owned(),
                request: request.clone(),
                timeout,
            });
            let _pass = self.gate.acquire().await;
            match self.answer {
                Answer::Clip => Ok(json!({"seconds": 1.5, "clip": STANDARD.encode(CLIP)})),
                Answer::Encoded(clip) => Ok(json!({"seconds": 1.5, "clip": clip})),
                Answer::Refuse(error) => Err(error),
                Answer::Malformed => Ok(json!({"length": 1})),
            }
        })
    }
}
