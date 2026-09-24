use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use backend_contracts::pipeline::{
    EncodeModel, EncodeRequest, EncodeResult, MAX_ENCODE_TEXT_BYTES, Producer,
};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::bus::nats::NatsService;
use crate::bus::subjects;
use crate::cache::CacheService;
use crate::cache::cache_service::CacheScope;
use crate::error::AppResult;
use crate::qdrant::{QdrantService, StoredQueryVector, collections};

pub const MAX_ENCODE_TEXT_CHARS: usize = MAX_ENCODE_TEXT_BYTES as usize / 4;

const VEC_CACHE_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const ENCODE_DEDUP_WINDOW_SECS: u64 = 15 * 60;
const FAILURE_ANSWER_SECS: i64 = 60;
const ENCODE_WAIT: Duration = Duration::from_secs(10);
const CACHE_RECHECK: Duration = Duration::from_millis(500);
const STORE_RECHECK: Duration = Duration::from_secs(2);
const RESUBSCRIBE_PAUSE: Duration = Duration::from_secs(1);
const LATE_RESULT_WAIT: Duration = Duration::from_secs(ENCODE_DEDUP_WINDOW_SECS);
const MAX_LATE_WATCHES: usize = 4096;
const EMPTY_MARKER: &str = "[]";

#[derive(Debug, Clone, PartialEq)]
pub enum EncodeOutcome {
    Ready(Vec<f32>),
    Declined {
        status: WorkerStatus,
        reason: Option<WorkerReason>,
    },
    Preparing,
}

impl EncodeOutcome {
    fn empty_text() -> Self {
        Self::Declined {
            status: WorkerStatus::Empty,
            reason: Some(WorkerReason::EmptyText),
        }
    }

    fn invalid_output() -> Self {
        Self::Declined {
            status: WorkerStatus::Failed,
            reason: Some(WorkerReason::ModelOutputInvalid),
        }
    }

    fn of_cached(vector: Vec<f32>) -> Self {
        if vector.is_empty() {
            Self::empty_text()
        } else {
            Self::Ready(vector)
        }
    }
}

struct EncodeTarget {
    model: EncodeModel,
    encoder: &'static str,
    prefix: &'static str,
    collection: &'static str,
}

const MULAN: EncodeTarget = EncodeTarget {
    model: EncodeModel::Mulan,
    encoder: "OpenMuQ/MuQ-MuLan-large",
    prefix: "vibe:vec:mulan:v1:",
    collection: collections::QUERY_VEC_MULAN,
};
const LYRICS: EncodeTarget = EncodeTarget {
    model: EncodeModel::Lyrics,
    encoder: "Qwen/Qwen3-Embedding-0.6B",
    prefix: "vibe:vec:lyrics:v2:",
    collection: collections::QUERY_VEC_LYRICS,
};

struct EncodeKeys {
    cache: String,
    failure: String,
    inflight: String,
}

impl EncodeKeys {
    fn new(target: &EncodeTarget, hash: &str) -> Self {
        let model = target.model.as_str();
        Self {
            cache: format!("{}{hash}", target.prefix),
            failure: format!("encfail:{model}:{hash}"),
            inflight: format!("encjob:{model}:{hash}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EncodeFailure {
    status: WorkerStatus,
    reason: Option<WorkerReason>,
    attempt: u32,
    failed_at: i64,
}

impl EncodeFailure {
    fn after(outcome: &EncodeOutcome, attempt: u32, now: i64) -> Option<Self> {
        match outcome {
            EncodeOutcome::Declined { status, reason } if *status != WorkerStatus::Empty => {
                Some(Self {
                    status: *status,
                    reason: *reason,
                    attempt,
                    failed_at: now,
                })
            }
            _ => None,
        }
    }

    fn answer_at(&self, now: i64) -> Option<EncodeOutcome> {
        (now.saturating_sub(self.failed_at) < FAILURE_ANSWER_SECS).then_some(
            EncodeOutcome::Declined {
                status: self.status,
                reason: self.reason,
            },
        )
    }

    fn next_attempt(failure: Option<&Self>) -> u32 {
        failure.map_or(0, |failure| failure.attempt.saturating_add(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    Published,
    HeldElsewhere,
    Unpublished,
}

trait EncodeStores {
    async fn cached(&self, key: &str) -> Option<EncodeOutcome>;
    async fn failure(&self, key: &str) -> Option<EncodeFailure>;
    async fn stored(&self, target: &EncodeTarget, hash: &str) -> Option<StoredQueryVector>;
    async fn remember(&self, key: &str, vector: &[f32]);
    async fn remember_failure(&self, key: &str, failure: &EncodeFailure);
    async fn claim(&self, inflight_key: &str) -> bool;
    async fn release(&self, inflight_key: &str);
    async fn publish(&self, request: &EncodeRequest, message_id: &str) -> AppResult<()>;
    fn results(&self) -> &Arc<ResultFeed>;
}

pub struct WorkerClient {
    nats: Arc<NatsService>,
    cache: Arc<CacheService>,
    qdrant: Arc<QdrantService>,
    results: Arc<ResultFeed>,
}

impl WorkerClient {
    pub fn new(
        nats: Arc<NatsService>,
        cache: Arc<CacheService>,
        qdrant: Arc<QdrantService>,
    ) -> Arc<Self> {
        let results = ResultFeed::new(nats.clone());
        Arc::new(Self {
            nats,
            cache,
            qdrant,
            results,
        })
    }

    pub async fn encode_text_mulan(&self, text: &str) -> AppResult<EncodeOutcome> {
        Ok(encode(self, &MULAN, text).await)
    }

    pub async fn encode_lyrics_text(&self, text: &str) -> AppResult<EncodeOutcome> {
        Ok(encode(self, &LYRICS, text).await)
    }

    async fn write(&self, key: &str, json: &str, ttl_secs: u64) {
        if let Err(error) = self
            .cache
            .set_raw(key, json, ttl_secs, None, CacheScope::Shared, None)
            .await
        {
            debug!(%error, key, "encode state was not cached");
        }
    }
}

impl EncodeStores for WorkerClient {
    async fn cached(&self, key: &str) -> Option<EncodeOutcome> {
        let raw = self.cache.get_raw(key).await.ok()??;
        let vector = serde_json::from_str::<Vec<f32>>(&raw).ok()?;
        Some(EncodeOutcome::of_cached(vector))
    }

    async fn failure(&self, key: &str) -> Option<EncodeFailure> {
        let raw = self.cache.get_raw(key).await.ok()??;
        serde_json::from_str(&raw).ok()
    }

    async fn stored(&self, target: &EncodeTarget, hash: &str) -> Option<StoredQueryVector> {
        self.qdrant.get_query_vector(target.collection, hash).await
    }

    async fn remember(&self, key: &str, vector: &[f32]) {
        let json = if vector.is_empty() {
            EMPTY_MARKER.to_owned()
        } else {
            match serde_json::to_string(vector) {
                Ok(json) => json,
                Err(error) => {
                    debug!(%error, "encoded vector could not be serialised for the cache");
                    return;
                }
            }
        };
        self.write(key, &json, VEC_CACHE_TTL_SECS).await;
    }

    async fn remember_failure(&self, key: &str, failure: &EncodeFailure) {
        match serde_json::to_string(failure) {
            Ok(json) => self.write(key, &json, ENCODE_DEDUP_WINDOW_SECS).await,
            Err(error) => debug!(%error, "encode failure could not be serialised for the cache"),
        }
    }

    async fn claim(&self, inflight_key: &str) -> bool {
        self.cache
            .try_acquire_lock(inflight_key, ENCODE_DEDUP_WINDOW_SECS)
            .await
            .unwrap_or(true)
    }

    async fn release(&self, inflight_key: &str) {
        if let Err(error) = self.cache.release_lock(inflight_key).await {
            debug!(%error, "encode in-flight lock was not released");
        }
    }

    async fn publish(&self, request: &EncodeRequest, message_id: &str) -> AppResult<()> {
        self.nats
            .publish_dedup(subjects::ENCODE_TEXT_NEW, request, message_id)
            .await
    }

    fn results(&self) -> &Arc<ResultFeed> {
        &self.results
    }
}

async fn encode<S: EncodeStores>(
    stores: &S,
    target: &'static EncodeTarget,
    text: &str,
) -> EncodeOutcome {
    let Some(text) = encode_text(text) else {
        return EncodeOutcome::empty_text();
    };
    settle_late_results(stores).await;
    let hash = text_hash(&text);
    let keys = EncodeKeys::new(target, &hash);
    if let Some(outcome) = stores.cached(&keys.cache).await {
        return outcome;
    }
    let failure = stores.failure(&keys.failure).await;
    if let Some(outcome) = failure
        .as_ref()
        .and_then(|failure| failure.answer_at(unix_now()))
    {
        return outcome;
    }
    if let Some(vector) = stored_vector(stores, target, &hash, &keys).await {
        return EncodeOutcome::Ready(vector);
    }

    let mut waiter = stores.results().watch(target.model, &hash).await;
    let attempt = EncodeFailure::next_attempt(failure.as_ref());
    let request = request_encoding(stores, target, &text, &hash, &keys, attempt).await;
    if request == Request::Unpublished {
        return EncodeOutcome::Preparing;
    }
    if request == Request::HeldElsewhere
        && let Some(vector) = stored_vector(stores, target, &hash, &keys).await
    {
        return EncodeOutcome::Ready(vector);
    }

    let pending = PendingEncode {
        target,
        hash: &hash,
        keys: &keys,
        attempt,
    };
    let outcome = await_encoding(stores, &pending, &mut waiter).await;
    if outcome == EncodeOutcome::Preparing
        && let Some(waiter) = waiter
    {
        stores.results().watch_late(LateWatch {
            target,
            hash: hash.clone(),
            attempt,
            until: Instant::now() + LATE_RESULT_WAIT,
        });
        drop(waiter);
    }
    settle_late_results(stores).await;
    outcome
}

async fn settle_late_results<S: EncodeStores>(stores: &S) {
    for (watch, result) in stores.results().late_results() {
        let keys = EncodeKeys::new(watch.target, &watch.hash);
        let pending = PendingEncode {
            target: watch.target,
            hash: &watch.hash,
            keys: &keys,
            attempt: watch.attempt,
        };
        settle(stores, &pending, result).await;
    }
}

async fn request_encoding<S: EncodeStores>(
    stores: &S,
    target: &EncodeTarget,
    text: &str,
    hash: &str,
    keys: &EncodeKeys,
    attempt: u32,
) -> Request {
    if !stores.claim(&keys.inflight).await {
        return Request::HeldElsewhere;
    }
    let request = EncodeRequest {
        model: target.model,
        text: text.to_owned(),
        hash: hash.to_owned(),
    };
    match stores
        .publish(&request, &message_id(target, hash, attempt))
        .await
    {
        Ok(()) => Request::Published,
        Err(error) => {
            warn!(%error, "encode job publish failed");
            stores.release(&keys.inflight).await;
            Request::Unpublished
        }
    }
}

async fn await_encoding<S: EncodeStores>(
    stores: &S,
    pending: &PendingEncode<'_>,
    waiter: &mut Option<ResultWaiter>,
) -> EncodeOutcome {
    let started = Instant::now();
    let deadline = started + ENCODE_WAIT;
    let mut next_recheck = started + CACHE_RECHECK;
    let mut next_store_check = started + STORE_RECHECK;
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => {
                return stored_vector(stores, pending.target, pending.hash, pending.keys)
                    .await
                    .map_or(EncodeOutcome::Preparing, EncodeOutcome::Ready);
            }
            () = tokio::time::sleep_until(next_recheck) => {
                next_recheck += CACHE_RECHECK;
                let look_in_store = Instant::now() >= next_store_check;
                if look_in_store {
                    next_store_check += STORE_RECHECK;
                }
                if let Some(outcome) = recheck(stores, pending, look_in_store).await {
                    return outcome;
                }
            }
            result = next_result(stores.results(), waiter) => match result {
                Some(result) => return settle(stores, pending, result).await,
                None => *waiter = None,
            },
        }
    }
}

async fn recheck<S: EncodeStores>(
    stores: &S,
    pending: &PendingEncode<'_>,
    look_in_store: bool,
) -> Option<EncodeOutcome> {
    if let Some(outcome) = stores.cached(&pending.keys.cache).await {
        return Some(outcome);
    }
    if let Some(outcome) = stores
        .failure(&pending.keys.failure)
        .await
        .and_then(|failure| failure.answer_at(unix_now()))
    {
        return Some(outcome);
    }
    if !look_in_store {
        return None;
    }
    stored_vector(stores, pending.target, pending.hash, pending.keys)
        .await
        .map(EncodeOutcome::Ready)
}

async fn settle<S: EncodeStores>(
    stores: &S,
    pending: &PendingEncode<'_>,
    result: EncodeResult,
) -> EncodeOutcome {
    stores
        .results()
        .forget_late(&result_key(pending.target.model, pending.hash));
    let outcome = outcome_of(pending.target, result);
    match &outcome {
        EncodeOutcome::Ready(vector) => {
            stores.remember(&pending.keys.cache, vector).await;
            stores.release(&pending.keys.inflight).await;
        }
        EncodeOutcome::Declined {
            status: WorkerStatus::Empty,
            ..
        } => {
            stores.remember(&pending.keys.cache, &[]).await;
            stores.release(&pending.keys.inflight).await;
        }
        EncodeOutcome::Declined { .. } => {
            if let Some(failure) = EncodeFailure::after(&outcome, pending.attempt, unix_now()) {
                stores
                    .remember_failure(&pending.keys.failure, &failure)
                    .await;
            }
            stores.release(&pending.keys.inflight).await;
        }
        EncodeOutcome::Preparing => {}
    }
    outcome
}

async fn stored_vector<S: EncodeStores>(
    stores: &S,
    target: &EncodeTarget,
    hash: &str,
    keys: &EncodeKeys,
) -> Option<Vec<f32>> {
    let stored = stores.stored(target, hash).await?;
    if !holds_vector_of(target, &stored) {
        debug!(
            model = target.model.as_str(),
            expected = target.encoder,
            stored = ?stored.encoder,
            "stored query vector is not from the expected encoder"
        );
        return None;
    }
    stores.remember(&keys.cache, &stored.vector).await;
    Some(stored.vector)
}

struct PendingEncode<'a> {
    target: &'static EncodeTarget,
    hash: &'a str,
    keys: &'a EncodeKeys,
    attempt: u32,
}

struct LateWatch {
    target: &'static EncodeTarget,
    hash: String,
    attempt: u32,
    until: Instant,
}

#[derive(Default)]
struct LateResults {
    watched: HashMap<String, LateWatch>,
    arrived: Vec<(LateWatch, EncodeResult)>,
}

impl LateResults {
    fn keeps_listening(&mut self, now: Instant) -> bool {
        self.watched.retain(|_, watch| watch.until > now);
        !self.watched.is_empty()
    }
}

type ResultStream = Pin<Box<dyn Stream<Item = Bytes> + Send>>;

trait ResultSource: Send + Sync {
    fn open_results(&self) -> BoxFuture<'_, AppResult<ResultStream>>;
}

impl ResultSource for NatsService {
    fn open_results(&self) -> BoxFuture<'_, AppResult<ResultStream>> {
        Box::pin(async move {
            let results = self.subscribe(subjects::DONE_ENCODE).await?;
            let payloads: ResultStream = Box::pin(results.map(|message| message.payload));
            Ok(payloads)
        })
    }
}

struct ResultFeed {
    source: Arc<dyn ResultSource>,
    board: ResultBoard,
    state: Mutex<FeedState>,
    late: Mutex<LateResults>,
    turn: Notify,
}

#[derive(Default)]
struct FeedState {
    parked: Option<ResultStream>,
    pumping: bool,
}

impl ResultFeed {
    fn new(source: Arc<dyn ResultSource>) -> Arc<Self> {
        Arc::new(Self {
            source,
            board: ResultBoard::default(),
            state: Mutex::new(FeedState::default()),
            late: Mutex::default(),
            turn: Notify::new(),
        })
    }

    async fn watch(self: &Arc<Self>, model: EncodeModel, hash: &str) -> Option<ResultWaiter> {
        let waiter = ResultWaiter::register(self, model, hash);
        if self.listening() {
            return Some(waiter);
        }
        match self.source.open_results().await {
            Ok(results) => {
                self.offer(results);
                Some(waiter)
            }
            Err(error) => {
                warn!(%error, "done.encode subscription failed, waiting on the cache alone");
                None
            }
        }
    }

    fn listening(&self) -> bool {
        let state = self.state();
        state.pumping || state.parked.is_some()
    }

    fn offer(&self, results: ResultStream) {
        let mut state = self.state();
        if !state.pumping && state.parked.is_none() {
            state.parked = Some(results);
        }
    }

    fn take_turn(&self) -> Option<Pump<'_>> {
        let mut state = self.state();
        if state.pumping {
            return None;
        }
        state.pumping = true;
        Some(Pump {
            feed: self,
            results: state.parked.take(),
        })
    }

    fn close_if_idle(&self) {
        let mut state = self.state();
        if !state.pumping && !self.keeps_listening() {
            state.parked = None;
        }
    }

    fn keeps_listening(&self) -> bool {
        !self.board.is_empty() || self.late().keeps_listening(Instant::now())
    }

    fn watch_late(&self, watch: LateWatch) {
        let key = result_key(watch.target.model, &watch.hash);
        let mut late = self.late();
        if late.watched.len() < MAX_LATE_WATCHES || late.watched.contains_key(&key) {
            late.watched.insert(key, watch);
        }
    }

    fn forget_late(&self, key: &str) {
        self.late().watched.remove(key);
    }

    fn late_results(&self) -> Vec<(LateWatch, EncodeResult)> {
        self.drain_buffered();
        std::mem::take(&mut self.late().arrived)
    }

    fn drain_buffered(&self) {
        let Some(mut pump) = self.take_turn() else {
            return;
        };
        while let Some(results) = pump.results.as_mut() {
            match results.next().now_or_never() {
                Some(Some(payload)) => self.deliver(&payload),
                Some(None) => pump.results = None,
                None => break,
            }
        }
    }

    fn deliver(&self, payload: &[u8]) {
        if self.board.deliver(payload) == Delivery::Unawaited {
            self.catch_late(payload);
        }
    }

    fn catch_late(&self, payload: &[u8]) {
        let mut late = self.late();
        if !late.keeps_listening(Instant::now()) {
            return;
        }
        let Ok(address) = serde_json::from_slice::<ResultAddress>(payload) else {
            return;
        };
        let key = result_key(address.model, &address.hash);
        if !late.watched.contains_key(&key) {
            return;
        }
        match serde_json::from_slice::<EncodeResult>(payload) {
            Ok(result) => {
                if let Some(watch) = late.watched.remove(&key) {
                    late.arrived.push((watch, result));
                }
            }
            Err(error) => debug!(%error, "late done.encode message is not an encode result"),
        }
    }

    async fn pump(&self, mut pump: Pump<'_>) {
        loop {
            match pump.results.as_mut() {
                Some(results) => match results.next().await {
                    Some(payload) => self.deliver(&payload),
                    None => pump.results = None,
                },
                None => match self.source.open_results().await {
                    Ok(results) => pump.results = Some(results),
                    Err(error) => {
                        warn!(%error, "done.encode subscription could not be renewed");
                        tokio::time::sleep(RESUBSCRIBE_PAUSE).await;
                    }
                },
            }
        }
    }

    fn state(&self) -> MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn late(&self) -> MutexGuard<'_, LateResults> {
        self.late.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Pump<'a> {
    feed: &'a ResultFeed,
    results: Option<ResultStream>,
}

impl Drop for Pump<'_> {
    fn drop(&mut self) {
        {
            let mut state = self.feed.state();
            state.pumping = false;
            let results = self.results.take();
            if self.feed.keeps_listening() {
                state.parked = results;
            }
        }
        self.feed.turn.notify_waiters();
    }
}

async fn next_result(feed: &ResultFeed, waiter: &mut Option<ResultWaiter>) -> Option<EncodeResult> {
    let Some(waiter) = waiter.as_mut() else {
        return std::future::pending().await;
    };
    loop {
        let turn = feed.turn.notified();
        tokio::pin!(turn);
        turn.as_mut().enable();
        if let Some(pump) = feed.take_turn() {
            return tokio::select! {
                biased;
                result = &mut waiter.result => result.ok(),
                () = feed.pump(pump) => None,
            };
        }
        tokio::select! {
            biased;
            result = &mut waiter.result => return result.ok(),
            () = &mut turn => {}
        }
    }
}

#[derive(Default)]
struct ResultBoard {
    waiters: Mutex<HashMap<String, Vec<oneshot::Sender<EncodeResult>>>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Unaddressed,
    Unawaited,
    Malformed,
    Delivered(usize),
}

#[derive(Deserialize)]
struct ResultAddress {
    model: EncodeModel,
    hash: String,
}

impl ResultBoard {
    fn register(&self, key: String) -> oneshot::Receiver<EncodeResult> {
        let (sender, receiver) = oneshot::channel();
        self.waiters().entry(key).or_default().push(sender);
        receiver
    }

    fn awaits(&self, key: &str) -> bool {
        self.waiters().contains_key(key)
    }

    fn is_empty(&self) -> bool {
        self.waiters().is_empty()
    }

    fn forget_closed(&self, key: &str) {
        let mut waiters = self.waiters();
        if let Some(senders) = waiters.get_mut(key) {
            senders.retain(|sender| !sender.is_closed());
            if senders.is_empty() {
                waiters.remove(key);
            }
        }
    }

    fn deliver(&self, payload: &[u8]) -> Delivery {
        let Ok(address) = serde_json::from_slice::<ResultAddress>(payload) else {
            debug!("done.encode message carries no model and hash");
            return Delivery::Unaddressed;
        };
        let key = result_key(address.model, &address.hash);
        if !self.awaits(&key) {
            return Delivery::Unawaited;
        }
        let result = match serde_json::from_slice::<EncodeResult>(payload) {
            Ok(result) => result,
            Err(error) => {
                debug!(%error, "done.encode message is not an encode result");
                return Delivery::Malformed;
            }
        };
        let senders = self.waiters().remove(&key).unwrap_or_default();
        let delivered = senders
            .into_iter()
            .map(|sender| sender.send(result.clone()).is_ok())
            .filter(|sent| *sent)
            .count();
        Delivery::Delivered(delivered)
    }

    fn waiters(&self) -> MutexGuard<'_, HashMap<String, Vec<oneshot::Sender<EncodeResult>>>> {
        self.waiters.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct ResultWaiter {
    feed: Arc<ResultFeed>,
    key: String,
    result: oneshot::Receiver<EncodeResult>,
}

impl ResultWaiter {
    fn register(feed: &Arc<ResultFeed>, model: EncodeModel, hash: &str) -> Self {
        let key = result_key(model, hash);
        let result = feed.board.register(key.clone());
        Self {
            feed: feed.clone(),
            key,
            result,
        }
    }
}

impl Drop for ResultWaiter {
    fn drop(&mut self) {
        self.result.close();
        self.feed.board.forget_closed(&self.key);
        self.feed.close_if_idle();
    }
}

fn result_key(model: EncodeModel, hash: &str) -> String {
    format!("{}:{hash}", model.as_str())
}

fn message_id(target: &EncodeTarget, hash: &str, attempt: u32) -> String {
    let first = result_key(target.model, hash);
    if attempt == 0 {
        first
    } else {
        format!("{first}:retry{attempt}")
    }
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn encode_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cut = trimmed
        .chars()
        .take(MAX_ENCODE_TEXT_CHARS)
        .collect::<String>();
    Some(cut.trim_end().to_owned())
}

fn text_hash(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn outcome_of(target: &EncodeTarget, result: EncodeResult) -> EncodeOutcome {
    if result.status != WorkerStatus::Ok {
        return EncodeOutcome::Declined {
            status: result.status,
            reason: result.reason,
        };
    }
    if !produced_by_expected_encoder(target, &result.producer) {
        warn!(
            model = target.model.as_str(),
            expected = target.encoder,
            produced = ?result.producer.models.get(target.model.as_str()),
            worker = %result.producer.worker_id,
            "encode result came from an unexpected encoder"
        );
        return EncodeOutcome::invalid_output();
    }
    match result.vector {
        Some(vector) if vector_fits(target, &vector) => EncodeOutcome::Ready(vector),
        _ => EncodeOutcome::invalid_output(),
    }
}

fn produced_by_expected_encoder(target: &EncodeTarget, producer: &Producer) -> bool {
    is_expected_encoder(
        target,
        producer
            .models
            .get(target.model.as_str())
            .map(String::as_str),
    )
}

fn holds_vector_of(target: &EncodeTarget, stored: &StoredQueryVector) -> bool {
    is_expected_encoder(target, stored.encoder.as_deref()) && vector_fits(target, &stored.vector)
}

fn is_expected_encoder(target: &EncodeTarget, reference: Option<&str>) -> bool {
    reference
        .and_then(|reference| reference.split('@').next())
        .is_some_and(|repository| repository == target.encoder)
}

fn vector_fits(target: &EncodeTarget, vector: &[f32]) -> bool {
    u64::try_from(vector.len()).is_ok_and(|length| length == target.model.dimensions())
        && vector.iter().all(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::channel::mpsc;

    use super::*;
    use crate::error::AppError;
    use serde_json::json;

    const QWEN: &str = "Qwen/Qwen3-Embedding-0.6B@97b0c614";
    const BGE_M3: &str = "BAAI/bge-m3@5617a9f6";

    fn producer(model: &str, reference: &str) -> serde_json::Value {
        json!({
            "worker_id": "gpu-main",
            "build": "2026.09.1+abc123",
            "models": { model: reference },
            "sync_version": null
        })
    }

    fn done(target: &EncodeTarget, hash: &str, body: serde_json::Value) -> Vec<u8> {
        let mut message = json!({ "model": target.model.as_str(), "hash": hash });
        if let (Some(message), Some(body)) = (message.as_object_mut(), body.as_object()) {
            message.extend(body.clone());
        }
        serde_json::to_vec(&message).expect("done.encode body")
    }

    fn ok_lyrics_payload(hash: &str, reference: &str, dimensions: usize) -> Vec<u8> {
        done(
            &LYRICS,
            hash,
            json!({
                "status": "ok",
                "vector": vec![0.03125_f32; dimensions],
                "producer": producer("lyrics", reference)
            }),
        )
    }

    fn ok_lyrics(hash: &str, reference: &str, dimensions: usize) -> EncodeResult {
        serde_json::from_slice(&ok_lyrics_payload(hash, reference, dimensions))
            .expect("an encode result")
    }

    fn failed_lyrics(hash: &str) -> Vec<u8> {
        done(
            &LYRICS,
            hash,
            json!({
                "status": "failed",
                "reason": "deadline_exceeded",
                "vector": null,
                "producer": producer("lyrics", QWEN)
            }),
        )
    }

    fn guard<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
        value.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[derive(Default)]
    struct FakeSource {
        live: Mutex<Vec<mpsc::UnboundedSender<Bytes>>>,
        opened: AtomicUsize,
    }

    impl FakeSource {
        fn send(&self, payload: Vec<u8>) {
            let payload = Bytes::from(payload);
            guard(&self.live).retain(|live| live.unbounded_send(payload.clone()).is_ok());
        }

        fn open_now(&self) -> usize {
            let mut live = guard(&self.live);
            live.retain(|live| !live.is_closed());
            live.len()
        }
    }

    impl ResultSource for FakeSource {
        fn open_results(&self) -> BoxFuture<'_, AppResult<ResultStream>> {
            Box::pin(async move {
                self.opened.fetch_add(1, Ordering::SeqCst);
                let (sender, receiver) = mpsc::unbounded();
                guard(&self.live).push(sender);
                let results: ResultStream = Box::pin(receiver);
                Ok(results)
            })
        }
    }

    struct FakeStores {
        cache: Mutex<HashMap<String, Vec<f32>>>,
        failures: Mutex<HashMap<String, EncodeFailure>>,
        stored: Mutex<HashMap<String, StoredQueryVector>>,
        locks: Mutex<HashSet<String>>,
        published: Mutex<Vec<String>>,
        publish_fails: bool,
        source: Arc<FakeSource>,
        results: Arc<ResultFeed>,
    }

    impl Default for FakeStores {
        fn default() -> Self {
            let source = Arc::new(FakeSource::default());
            Self {
                cache: Mutex::default(),
                failures: Mutex::default(),
                stored: Mutex::default(),
                locks: Mutex::default(),
                published: Mutex::default(),
                publish_fails: false,
                results: ResultFeed::new(source.clone()),
                source,
            }
        }
    }

    impl FakeStores {
        fn keep(&self, hash: &str, reference: Option<&str>, dimensions: usize) {
            guard(&self.stored).insert(
                hash.to_owned(),
                StoredQueryVector {
                    vector: vec![0.03125; dimensions],
                    encoder: reference.map(str::to_owned),
                },
            );
        }

        fn published(&self) -> Vec<String> {
            guard(&self.published).clone()
        }

        fn locked(&self, key: &str) -> bool {
            guard(&self.locks).contains(key)
        }

        fn watched_late(&self) -> usize {
            self.results.late().watched.len()
        }
    }

    impl EncodeStores for FakeStores {
        async fn cached(&self, key: &str) -> Option<EncodeOutcome> {
            guard(&self.cache)
                .get(key)
                .cloned()
                .map(EncodeOutcome::of_cached)
        }

        async fn failure(&self, key: &str) -> Option<EncodeFailure> {
            guard(&self.failures).get(key).cloned()
        }

        async fn stored(&self, _target: &EncodeTarget, hash: &str) -> Option<StoredQueryVector> {
            guard(&self.stored)
                .get(hash)
                .map(|stored| StoredQueryVector {
                    vector: stored.vector.clone(),
                    encoder: stored.encoder.clone(),
                })
        }

        async fn remember(&self, key: &str, vector: &[f32]) {
            guard(&self.cache).insert(key.to_owned(), vector.to_vec());
        }

        async fn remember_failure(&self, key: &str, failure: &EncodeFailure) {
            guard(&self.failures).insert(key.to_owned(), failure.clone());
        }

        async fn claim(&self, inflight_key: &str) -> bool {
            guard(&self.locks).insert(inflight_key.to_owned())
        }

        async fn release(&self, inflight_key: &str) {
            guard(&self.locks).remove(inflight_key);
        }

        async fn publish(&self, _request: &EncodeRequest, message_id: &str) -> AppResult<()> {
            if self.publish_fails {
                return Err(AppError::internal("jetstream publish encode.text.new"));
            }
            guard(&self.published).push(message_id.to_owned());
            Ok(())
        }

        fn results(&self) -> &Arc<ResultFeed> {
            &self.results
        }
    }

    async fn answer_after(source: &FakeSource, after: Duration, payload: Vec<u8>) {
        tokio::time::sleep(after).await;
        source.send(payload);
    }

    fn board_only() -> Arc<ResultFeed> {
        ResultFeed::new(Arc::new(FakeSource::default()))
    }

    #[test]
    fn the_text_is_trimmed_and_cut_to_the_contract_limit_before_it_is_hashed() {
        assert_eq!(encode_text(" \n\t "), None);
        assert_eq!(encode_text("  rain  ").as_deref(), Some("rain"));

        let long = "я".repeat(MAX_ENCODE_TEXT_CHARS + 70);
        let cut = encode_text(&long).expect("a long query is still a query");
        assert_eq!(cut.chars().count(), MAX_ENCODE_TEXT_CHARS);
        assert_eq!(
            text_hash(&cut),
            text_hash(&encode_text(&"я".repeat(MAX_ENCODE_TEXT_CHARS + 1)).expect("query")),
            "two queries that differ only past the limit are one encoding"
        );

        let widest = "𝄞".repeat(MAX_ENCODE_TEXT_CHARS * 2);
        let cut = encode_text(&widest).expect("query");
        assert!(cut.len() <= MAX_ENCODE_TEXT_BYTES as usize);

        let spaced = format!("{} tail", "a".repeat(MAX_ENCODE_TEXT_CHARS - 1));
        assert_eq!(
            encode_text(&spaced),
            Some("a".repeat(MAX_ENCODE_TEXT_CHARS - 1)),
            "a cut that ends on a space does not hash the space"
        );
    }

    #[test]
    fn the_request_carries_the_hash_of_exactly_the_text_it_sends() {
        let text = encode_text(&"ночь ".repeat(60)).expect("query");
        let request = EncodeRequest {
            model: LYRICS.model,
            text: text.clone(),
            hash: text_hash(&text),
        };
        let wire = serde_json::to_value(&request).expect("request");

        assert_eq!(wire["model"], "lyrics");
        assert_eq!(
            wire["hash"],
            hex::encode(Sha256::digest(
                wire["text"].as_str().expect("text").as_bytes()
            ))
        );
    }

    #[test]
    fn every_waiter_for_a_text_gets_the_one_result_and_no_other_text_does() {
        let feed = board_only();
        let hash = text_hash("rain");
        let other = text_hash("snow");
        let mut first = ResultWaiter::register(&feed, EncodeModel::Lyrics, &hash);
        let mut second = ResultWaiter::register(&feed, EncodeModel::Lyrics, &hash);
        let mut unrelated = ResultWaiter::register(&feed, EncodeModel::Lyrics, &other);
        let mut mulan = ResultWaiter::register(&feed, EncodeModel::Mulan, &hash);

        assert_eq!(
            feed.board.deliver(&ok_lyrics_payload(&hash, QWEN, 1024)),
            Delivery::Delivered(2)
        );
        assert_eq!(first.result.try_recv().expect("first").hash, hash);
        assert_eq!(second.result.try_recv().expect("second").hash, hash);
        assert!(unrelated.result.try_recv().is_err());
        assert!(mulan.result.try_recv().is_err());
        assert!(!feed.board.awaits(&result_key(EncodeModel::Lyrics, &hash)));
        assert_eq!(
            feed.board.deliver(b"{\"model\":\"lyrics\"}"),
            Delivery::Unaddressed
        );
    }

    #[test]
    fn a_result_nobody_waits_for_is_not_parsed_past_its_address() {
        let feed = board_only();
        let hash = text_hash("rain");
        let heavy = done(
            &LYRICS,
            &hash,
            json!({ "status": "ok", "vector": "not a vector", "producer": {} }),
        );

        assert_eq!(feed.board.deliver(&heavy), Delivery::Unawaited);
        let _waiter = ResultWaiter::register(&feed, EncodeModel::Lyrics, &hash);
        assert_eq!(
            feed.board.deliver(&heavy),
            Delivery::Malformed,
            "only a result someone waits for is parsed in full"
        );
    }

    #[test]
    fn a_waiter_that_gives_up_leaves_nothing_on_the_board() {
        let feed = board_only();
        let hash = text_hash("rain");
        let key = result_key(EncodeModel::Lyrics, &hash);
        let staying = ResultWaiter::register(&feed, EncodeModel::Lyrics, &hash);
        drop(ResultWaiter::register(&feed, EncodeModel::Lyrics, &hash));
        assert!(feed.board.awaits(&key));
        drop(staying);
        assert!(!feed.board.awaits(&key));
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_waiters_share_one_subscription_that_closes_when_nobody_waits() {
        let stores = FakeStores::default();
        let rain = text_hash("rain");
        let snow = text_hash("snow");

        let (first, second, (), ()) = tokio::join!(
            encode(&stores, &LYRICS, "rain"),
            encode(&stores, &LYRICS, "snow"),
            answer_after(
                &stores.source,
                Duration::from_secs(1),
                ok_lyrics_payload(&rain, QWEN, 1024)
            ),
            answer_after(
                &stores.source,
                Duration::from_secs(2),
                ok_lyrics_payload(&snow, QWEN, 1024)
            ),
        );

        assert!(matches!(first, EncodeOutcome::Ready(vector) if vector.len() == 1024));
        assert!(
            matches!(second, EncodeOutcome::Ready(vector) if vector.len() == 1024),
            "the second waiter lost its result when the first one stopped reading"
        );
        assert_eq!(stores.source.opened.load(Ordering::SeqCst), 1);
        assert_eq!(
            stores.source.open_now(),
            0,
            "an idle subscription keeps buffering every result on the subject"
        );
    }

    #[test]
    fn a_vector_from_the_expected_encoder_is_ready() {
        let hash = text_hash("rain");
        let result = ok_lyrics(&hash, QWEN, 1024);

        assert!(matches!(
            outcome_of(&LYRICS, result),
            EncodeOutcome::Ready(vector) if vector.len() == 1024
        ));
    }

    #[test]
    fn a_vector_from_another_encoder_or_of_the_wrong_size_is_refused() {
        let hash = text_hash("rain");

        assert_eq!(
            outcome_of(&LYRICS, ok_lyrics(&hash, BGE_M3, 1024)),
            EncodeOutcome::invalid_output(),
            "bge-m3 answers in the same 1024 dimensions and a different space"
        );
        assert_eq!(
            outcome_of(&LYRICS, ok_lyrics(&hash, QWEN, 512)),
            EncodeOutcome::invalid_output()
        );
        let mut poisoned = ok_lyrics(&hash, QWEN, 1024);
        if let Some(vector) = poisoned.vector.as_mut() {
            vector[7] = f32::NAN;
        }
        assert_eq!(
            outcome_of(&LYRICS, poisoned),
            EncodeOutcome::invalid_output()
        );
    }

    #[test]
    fn a_refusal_carries_the_status_and_reason_the_worker_gave() {
        let hash = text_hash("...");
        let empty = done(
            &LYRICS,
            &hash,
            json!({
                "status": "empty",
                "reason": "empty_text",
                "vector": null,
                "producer": producer("lyrics", QWEN)
            }),
        );
        let failed = done(
            &MULAN,
            &hash,
            json!({
                "status": "failed",
                "reason": "hash_mismatch",
                "detail": "sha256 differs",
                "vector": null,
                "producer": producer("mulan", "OpenMuQ/MuQ-MuLan-large@2e01c796")
            }),
        );

        assert_eq!(
            outcome_of(
                &LYRICS,
                serde_json::from_slice(&empty).expect("empty result")
            ),
            EncodeOutcome::empty_text()
        );
        assert_eq!(
            outcome_of(
                &MULAN,
                serde_json::from_slice(&failed).expect("failed result")
            ),
            EncodeOutcome::Declined {
                status: WorkerStatus::Failed,
                reason: Some(WorkerReason::HashMismatch),
            }
        );
    }

    #[test]
    fn lyrics_vectors_live_under_a_prefix_no_bge_m3_vector_was_written_to() {
        assert_eq!(LYRICS.prefix, "vibe:vec:lyrics:v2:");
        assert_eq!(LYRICS.collection, collections::QUERY_VEC_LYRICS);
        assert_eq!(MULAN.collection, collections::QUERY_VEC_MULAN);
        assert_ne!(LYRICS.prefix, MULAN.prefix);
    }

    #[test]
    fn a_stored_vector_counts_only_when_its_point_names_the_expected_encoder() {
        let stored = |encoder: Option<&str>, dimensions: usize| StoredQueryVector {
            vector: vec![0.03125; dimensions],
            encoder: encoder.map(str::to_owned),
        };

        assert!(holds_vector_of(&LYRICS, &stored(Some(QWEN), 1024)));
        assert!(!holds_vector_of(&LYRICS, &stored(Some(BGE_M3), 1024)));
        assert!(
            !holds_vector_of(&LYRICS, &stored(None, 1024)),
            "a point that does not say who encoded it may be from any space"
        );
        assert!(!holds_vector_of(&LYRICS, &stored(Some(QWEN), 512)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_point_written_from_another_encoder_is_never_served_or_cached() {
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);
        let stores = FakeStores {
            publish_fails: true,
            ..FakeStores::default()
        };
        stores.keep(&hash, Some(BGE_M3), 1024);

        assert_eq!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Preparing
        );
        assert!(!guard(&stores.cache).contains_key(&keys.cache));

        stores.keep(&hash, Some(QWEN), 1024);
        assert!(matches!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Ready(vector) if vector.len() == 1024
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_encoding_is_answered_at_once_afterwards_and_frees_its_lock() {
        let stores = FakeStores::default();
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);
        let deadline_exceeded = EncodeOutcome::Declined {
            status: WorkerStatus::Failed,
            reason: Some(WorkerReason::DeadlineExceeded),
        };

        let (first, ()) = tokio::join!(
            encode(&stores, &LYRICS, "rain"),
            answer_after(&stores.source, Duration::from_secs(1), failed_lyrics(&hash)),
        );
        assert_eq!(first, deadline_exceeded);
        assert!(!stores.locked(&keys.inflight));

        let started = Instant::now();
        assert_eq!(encode(&stores, &LYRICS, "rain").await, deadline_exceeded);
        assert!(
            started.elapsed() < CACHE_RECHECK,
            "a known failure waited {:?} for an answer that will never come",
            started.elapsed()
        );
        assert_eq!(stores.published(), vec![format!("lyrics:{hash}")]);
    }

    #[tokio::test(start_paused = true)]
    async fn an_old_failure_is_retried_under_a_message_id_the_dedup_window_does_not_swallow() {
        let stores = FakeStores::default();
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);
        guard(&stores.failures).insert(
            keys.failure.clone(),
            EncodeFailure {
                status: WorkerStatus::Failed,
                reason: Some(WorkerReason::DeadlineExceeded),
                attempt: 0,
                failed_at: unix_now() - FAILURE_ANSWER_SECS - 1,
            },
        );

        assert_eq!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Preparing
        );
        assert_eq!(stores.published(), vec![format!("lyrics:{hash}:retry1")]);
        assert_eq!(message_id(&LYRICS, &hash, 0), format!("lyrics:{hash}"));
    }

    #[test]
    fn only_a_refusal_that_is_not_empty_text_is_remembered_as_a_failure() {
        let failed = EncodeOutcome::invalid_output();
        let failure = EncodeFailure::after(&failed, 2, 1_000).expect("a failure");

        assert_eq!(
            failure.answer_at(1_000 + FAILURE_ANSWER_SECS - 1),
            Some(failed)
        );
        assert_eq!(failure.answer_at(1_000 + FAILURE_ANSWER_SECS), None);
        assert_eq!(EncodeFailure::next_attempt(Some(&failure)), 3);
        assert_eq!(
            EncodeFailure::after(&EncodeOutcome::empty_text(), 0, 1_000),
            None
        );
        assert_eq!(
            EncodeFailure::after(&EncodeOutcome::Ready(vec![1.0]), 0, 1_000),
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_that_was_never_published_is_not_waited_for() {
        let stores = FakeStores {
            publish_fails: true,
            ..FakeStores::default()
        };
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);

        let started = Instant::now();
        assert_eq!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Preparing
        );
        assert!(
            started.elapsed() < CACHE_RECHECK,
            "an unpublished job was waited on for {:?}",
            started.elapsed()
        );
        assert!(!stores.locked(&keys.inflight));
        assert!(stores.results.board.is_empty());
        assert_eq!(stores.source.open_now(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_vector_that_reaches_the_store_while_waiting_is_taken_from_there() {
        for lands_after in [Duration::from_secs(3), Duration::from_millis(9_600)] {
            let stores = FakeStores::default();
            let hash = text_hash("rain");
            let keys = EncodeKeys::new(&LYRICS, &hash);
            guard(&stores.locks).insert(keys.inflight.clone());
            let land = async {
                tokio::time::sleep(lands_after).await;
                stores.keep(&hash, Some(QWEN), 1024);
            };

            let (outcome, ()) = tokio::join!(encode(&stores, &LYRICS, "rain"), land);

            assert!(
                matches!(&outcome, EncodeOutcome::Ready(vector) if vector.len() == 1024),
                "a vector stored after {lands_after:?} was missed: {outcome:?}"
            );
            assert!(stores.published().is_empty());
            assert!(guard(&stores.cache).contains_key(&keys.cache));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_vector_that_arrives_in_time_frees_its_lock() {
        let stores = FakeStores::default();
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);

        let (outcome, ()) = tokio::join!(
            encode(&stores, &LYRICS, "rain"),
            answer_after(
                &stores.source,
                Duration::from_secs(1),
                ok_lyrics_payload(&hash, QWEN, 1024)
            ),
        );

        assert!(matches!(outcome, EncodeOutcome::Ready(vector) if vector.len() == 1024));
        assert!(!stores.locked(&keys.inflight));
        assert_eq!(stores.watched_late(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_vector_that_arrives_after_everyone_gave_up_is_served_from_the_cache() {
        let stores = FakeStores::default();
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);

        assert_eq!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Preparing
        );
        assert_eq!(stores.watched_late(), 1);
        assert_eq!(stores.source.open_now(), 1);
        answer_after(
            &stores.source,
            Duration::from_secs(20),
            ok_lyrics_payload(&hash, QWEN, 1024),
        )
        .await;

        let started = Instant::now();
        assert!(matches!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Ready(vector) if vector.len() == 1024
        ));
        assert!(
            started.elapsed() < CACHE_RECHECK,
            "a vector that already arrived was waited on for {:?}",
            started.elapsed()
        );
        assert!(!stores.locked(&keys.inflight));
        assert_eq!(stores.published(), vec![format!("lyrics:{hash}")]);
        assert_eq!(stores.watched_late(), 0);
        assert_eq!(stores.source.open_now(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_that_arrives_after_everyone_gave_up_is_remembered_and_frees_the_lock() {
        let stores = FakeStores::default();
        let hash = text_hash("rain");
        let keys = EncodeKeys::new(&LYRICS, &hash);
        let deadline_exceeded = EncodeOutcome::Declined {
            status: WorkerStatus::Failed,
            reason: Some(WorkerReason::DeadlineExceeded),
        };

        assert_eq!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Preparing
        );
        answer_after(
            &stores.source,
            Duration::from_secs(30),
            failed_lyrics(&hash),
        )
        .await;
        assert!(stores.locked(&keys.inflight));
        assert_eq!(
            encode(&stores, &LYRICS, "snow").await,
            EncodeOutcome::Preparing,
            "a request for any other text settles what arrived late"
        );

        assert!(!stores.locked(&keys.inflight));
        assert_eq!(
            guard(&stores.failures)
                .get(&keys.failure)
                .map(|failure| failure.attempt),
            Some(0)
        );
        let started = Instant::now();
        assert_eq!(encode(&stores, &LYRICS, "rain").await, deadline_exceeded);
        assert!(started.elapsed() < CACHE_RECHECK);
        assert_eq!(
            stores.published(),
            vec![
                format!("lyrics:{hash}"),
                format!("lyrics:{}", text_hash("snow"))
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_result_later_than_the_lock_is_ignored_and_the_subscription_closes() {
        let stores = FakeStores::default();
        let hash = text_hash("rain");

        assert_eq!(
            encode(&stores, &LYRICS, "rain").await,
            EncodeOutcome::Preparing
        );
        tokio::time::sleep(LATE_RESULT_WAIT - Duration::from_secs(1)).await;
        assert!(stores.results.late_results().is_empty());
        assert_eq!(stores.source.open_now(), 1);

        tokio::time::sleep(Duration::from_secs(1)).await;
        answer_after(
            &stores.source,
            Duration::ZERO,
            ok_lyrics_payload(&hash, QWEN, 1024),
        )
        .await;
        assert!(stores.results.late_results().is_empty());
        assert_eq!(stores.watched_late(), 0);
        assert_eq!(stores.source.open_now(), 0);
        assert!(!guard(&stores.cache).contains_key(&EncodeKeys::new(&LYRICS, &hash).cache));
    }

    #[tokio::test(start_paused = true)]
    async fn late_results_are_watched_once_per_text_and_up_to_a_bound() {
        let stores = FakeStores::default();

        for _ in 0..2 {
            assert_eq!(
                encode(&stores, &LYRICS, "rain").await,
                EncodeOutcome::Preparing
            );
        }
        assert_eq!(stores.watched_late(), 1);

        for index in 1..MAX_LATE_WATCHES {
            stores.results.watch_late(LateWatch {
                target: &LYRICS,
                hash: text_hash(&format!("text {index}")),
                attempt: 0,
                until: Instant::now() + LATE_RESULT_WAIT,
            });
        }
        assert_eq!(stores.watched_late(), MAX_LATE_WATCHES);
        assert_eq!(
            encode(&stores, &LYRICS, "snow").await,
            EncodeOutcome::Preparing
        );
        assert_eq!(stores.watched_late(), MAX_LATE_WATCHES);
        assert!(
            !stores
                .results
                .late()
                .watched
                .contains_key(&result_key(EncodeModel::Lyrics, &text_hash("snow")))
        );
    }
}
