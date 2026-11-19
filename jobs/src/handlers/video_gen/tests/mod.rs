mod budget;
mod flow;
mod queue;

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use backend_contracts::pipeline::{Producer, VideoClip, VideoPart};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use backend_contracts::video_gen::{VideoFileSlice, VideoGenResult};
use backend_contracts::worker_contract::WorkerLane;
use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use url::Url;

use super::director::{Brief, Character, Director, Look, Plan, PlannedShot, Review, Verdict};
use super::lanes::Lanes;
use super::{Store, VideoGen};
use crate::config::{AnthropicLadder, VideoGenConfig};
use crate::handlers::video_accounts::Notify;
use crate::llm::anthropic::{Answer, LlmError, Spend};
use crate::objects::{Blobs, Listed, ObjectsError};

pub(super) type Bench = VideoGen<Scripted, Memory, Wire, Inbox>;
pub(super) type Seen = (Look, usize, usize, usize);

pub(super) const TRACK: &str = "501";
pub(super) const PLAN_USD: f64 = 0.30;
pub(super) const REVIEW_USD: f64 = 0.05;

pub(super) fn config() -> VideoGenConfig {
    VideoGenConfig {
        in_flight: 2,
        tick: Duration::from_secs(30),
        timeout: Duration::from_secs(3_600),
        lease: Duration::from_secs(900),
        month_usd: 50.0,
        recheck: Duration::from_secs(1_800),
        retry_base: Duration::from_secs(900),
        max_faults: 3,
        shots: 2,
        keyframe_tries: 3,
        shot_tries: 2,
        side: 720,
        fps: 16,
        frames: 81,
        stills: 4,
        director: Some(AnthropicLadder {
            steps: Vec::new(),
            reset_day: 17,
        }),
    }
}

pub(super) fn bench(
    pool: &PgPool,
    config: VideoGenConfig,
) -> (Bench, Scripted, Memory, Wire, Inbox) {
    let director = Scripted::default();
    let store = Memory::default();
    let wire = Wire::default();
    let inbox = Inbox::default();
    let bench = VideoGen::new(
        pool.clone(),
        Url::parse("https://storage.example/").unwrap(),
        config,
        director.clone(),
        store.clone(),
        wire.clone(),
        inbox.clone(),
    );
    (bench, director, store, wire, inbox)
}

pub(super) fn plan() -> Plan {
    Plan {
        character: Character {
            gender: "woman".to_owned(),
            hair: "long black hair".to_owned(),
            clothes: "red coat".to_owned(),
            face_visible: true,
        },
        mood: "night rain".to_owned(),
        bans: vec!["other people".to_owned()],
        shots: vec![
            PlannedShot {
                start_ms: 30_000,
                end_ms: 34_000,
                line: "first line".to_owned(),
                keyframe: "she stands under a street lamp".to_owned(),
                motion: "slow push in".to_owned(),
            },
            PlannedShot {
                start_ms: 34_000,
                end_ms: 38_500,
                line: "second line".to_owned(),
                keyframe: "close up, rain on her face".to_owned(),
                motion: "she turns her head".to_owned(),
            },
        ],
    }
}

#[derive(Clone, Default)]
pub(super) struct Scripted {
    plans: Arc<Mutex<VecDeque<Result<Plan, LlmError>>>>,
    verdicts: Arc<Mutex<VecDeque<Result<Verdict, LlmError>>>>,
    pub(super) briefs: Arc<Mutex<Vec<Brief>>>,
    pub(super) reviews: Arc<Mutex<Vec<Seen>>>,
}

impl Scripted {
    pub(super) fn plans(&self, answer: Result<Plan, LlmError>) {
        self.plans.lock().unwrap().push_back(answer);
    }

    pub(super) fn verdicts(&self, answer: Result<Verdict, LlmError>) {
        self.verdicts.lock().unwrap().push_back(answer);
    }
}

fn spent<T>(call: &'static str, usd: f64, result: Result<T, LlmError>) -> Answer<T> {
    let spend = match &result {
        Err(LlmError::Down(_)) => Vec::new(),
        _ => vec![Spend {
            call,
            provider: "opus".to_owned(),
            model: "example-model".to_owned(),
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            usd,
        }],
    };
    Answer { result, spend }
}

impl Director for Scripted {
    async fn plan(&self, brief: &Brief) -> Answer<Plan> {
        self.briefs.lock().unwrap().push(brief.clone());
        let next = self.plans.lock().unwrap().pop_front();
        spent(
            "plan",
            PLAN_USD,
            next.unwrap_or(Err(LlmError::Invalid("unscripted".to_owned()))),
        )
    }

    async fn review(&self, review: &Review<'_>) -> Answer<Verdict> {
        self.reviews.lock().unwrap().push((
            review.look,
            review.shot,
            review.current.len(),
            review.earlier.len(),
        ));
        let next = self.verdicts.lock().unwrap().pop_front();
        spent(
            "review",
            REVIEW_USD,
            next.unwrap_or(Err(LlmError::Invalid("unscripted".to_owned()))),
        )
    }
}

#[derive(Clone, Default)]
pub(super) struct Memory {
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl Memory {
    pub(super) fn keys(&self) -> Vec<String> {
        self.files.lock().unwrap().keys().cloned().collect()
    }
}

impl Blobs for Memory {
    async fn put_file(&self, key: &str, path: &Path) -> Result<(), ObjectsError> {
        let bytes = std::fs::read(path).unwrap();
        self.files.lock().unwrap().insert(key.to_owned(), bytes);
        Ok(())
    }

    async fn put_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectsError> {
        self.files.lock().unwrap().insert(key.to_owned(), bytes);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectsError> {
        self.files.lock().unwrap().remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<Listed>, ObjectsError> {
        Ok(self
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(prefix))
            .map(|key| Listed {
                key,
                modified: SystemTime::now(),
            })
            .collect())
    }
}

impl Store for Memory {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectsError> {
        Ok(self.files.lock().unwrap().get(key).cloned())
    }
}

#[derive(Clone, Default)]
pub(super) struct Wire {
    pub(super) sent: Arc<Mutex<Vec<(String, Value, String)>>>,
    pub(super) down: Arc<AtomicBool>,
}

impl Wire {
    pub(super) fn last(&self) -> (String, Value, String) {
        self.sent.lock().unwrap().last().cloned().unwrap()
    }

    pub(super) fn subjects(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(subject, _, _)| subject.clone())
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

pub(super) fn answered(
    request: &Value,
    files: &[(&str, &[u8])],
    part_bytes: usize,
) -> Vec<VideoGenResult> {
    let blob: Vec<u8> = files
        .iter()
        .flat_map(|(_, data)| data.iter().copied())
        .collect();
    let mut slices = Vec::new();
    let mut offset = 0;
    for (name, data) in files {
        slices.push(VideoFileSlice {
            name: (*name).to_owned(),
            offset,
            bytes: data.len() as u64,
        });
        offset += data.len() as u64;
    }
    let chunks: Vec<&[u8]> = blob.chunks(part_bytes).collect();
    let clip = VideoClip {
        bytes: blob.len() as u64,
        sha256: Sha256::digest(&blob)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        parts: chunks.len() as u32,
    };
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| VideoGenResult {
            clip: Some(clip.clone()),
            files: Some(slices.clone()),
            part: Some(VideoPart {
                index: index as u32,
                data: base64::engine::general_purpose::STANDARD.encode(chunk),
            }),
            ..failed(request, WorkerStatus::Ok, None)
        })
        .collect()
}

pub(super) fn failed(
    request: &Value,
    status: WorkerStatus,
    reason: Option<WorkerReason>,
) -> VideoGenResult {
    VideoGenResult {
        sc_track_id: request["sc_track_id"].as_str().unwrap().to_owned(),
        attempt: request["attempt"].as_i64().unwrap(),
        step: request["step"].as_i64().unwrap(),
        status,
        reason,
        detail: reason.map(|_| "test".to_owned()),
        producer: Producer {
            worker_id: "sc-gpu1".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: None,
        },
        clip: None,
        files: None,
        part: None,
    }
}

pub(super) async fn track(pool: &PgPool, id: &str, uploader: &str) {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms,
                             storage_state, metadata_artist, artwork_url, uploader_sc_user_id)
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 'public', 200000, 'ok', 'Artist',
                 'https://i1.sndcdn.com/artworks-' || $1 || '-large.jpg', $2)",
    )
    .bind(id)
    .bind(uploader)
    .execute(pool)
    .await
    .unwrap();
}

pub(super) async fn star(pool: &PgPool, user: &str) {
    sqlx::query("INSERT INTO subscriptions (user_urn, exp_date) VALUES ($1, extract(epoch FROM now())::bigint + 86400)")
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn superlike(pool: &PgPool, user: &str, track: &str) {
    sqlx::query("INSERT INTO user_superlikes (user_id, sc_track_id, like_created_at) VALUES ($1, $2, now())")
        .bind(user)
        .bind(track)
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn flag(pool: &PgPool, kind: &str, user: &str) {
    sqlx::query("INSERT INTO video_flags (kind, sc_user_id, added_by) VALUES ($1, $2, 'admin')")
        .bind(kind)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn queued(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT sc_track_id FROM track_video_gen ORDER BY sc_track_id")
        .fetch_all(pool)
        .await
        .unwrap()
}

pub(super) async fn row(pool: &PgPool, track: &str) -> (String, Option<String>, Value, i32) {
    sqlx::query_as(
        "SELECT stage, reason, rejections, faults FROM track_video_gen WHERE sc_track_id = $1",
    )
    .bind(track)
    .fetch_one(pool)
    .await
    .unwrap()
}

pub(super) async fn reply(
    bench: &Bench,
    wire: &Wire,
    lane: WorkerLane,
    files: &[(&str, &[u8])],
    part_bytes: usize,
) {
    let (_, request, _) = wire.last();
    for result in answered(&request, files, part_bytes) {
        bench.finish(lane, result).await.unwrap();
    }
}

pub(super) async fn lyrics(pool: &PgPool, track: &str) {
    sqlx::query("INSERT INTO lyrics_cache (sc_track_id, source, synced_lrc) VALUES ($1, 'lrclib', '[00:30.00] first line')")
        .bind(track)
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn due_now(pool: &PgPool) {
    sqlx::query("UPDATE track_video_gen SET next_at = now()")
        .execute(pool)
        .await
        .unwrap();
}
