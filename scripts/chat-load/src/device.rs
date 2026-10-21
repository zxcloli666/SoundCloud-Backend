use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep, sleep_until};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::{CallError, Client, Clock, Reply};
use crate::b64;
use crate::content::{self, Sealing, Stamp};
use crate::layout::Identity;
use crate::stats::Stats;

pub const HOLD_SLACK: Duration = Duration::from_secs(15);
pub const CALL_TIMEOUT: Duration = Duration::from_secs(20);
pub const BACKOFF_MAX_S: u64 = 15;
pub const RECONNECT_JITTER_MS: u64 = 10_000;

pub struct Behavior {
    pub hold_ms: u64,
    pub short_every: Duration,
    pub send_every: Option<Duration>,
    pub type_every: Option<Duration>,
    pub run_id: u32,
}

pub struct Shared {
    pub stats: Arc<Stats>,
    pub behavior: Behavior,
    pub all_parked: AtomicBool,
    pub clock: Arc<Clock>,
}

#[derive(Default)]
struct State {
    pos: u64,
    inc: Option<Uuid>,
    known: HashMap<Uuid, i64>,
    epochs: HashMap<Uuid, i32>,
    keys: HashMap<(Uuid, i32), [u8; 32]>,
    marks: HashMap<Uuid, i64>,
}

pub struct Device {
    identity: Arc<Identity>,
    route: usize,
    client: Client,
    chats: Vec<Uuid>,
    parked: bool,
    focus: Option<Uuid>,
    shared: Arc<Shared>,
    state: Mutex<State>,
    recovered: AtomicBool,
}

pub struct Mode {
    pub parked: bool,
    pub focused: bool,
}

struct ParkedGauge<'a> {
    stats: &'a Stats,
    route: usize,
}

impl<'a> ParkedGauge<'a> {
    fn new(stats: &'a Stats, route: usize) -> Self {
        stats.parked(route, 1);
        Self { stats, route }
    }
}

impl Drop for ParkedGauge<'_> {
    fn drop(&mut self) {
        self.stats.parked(self.route, -1);
    }
}

impl Device {
    pub fn new(
        identity: Arc<Identity>,
        route: usize,
        client: Client,
        chats: Vec<Uuid>,
        mode: Mode,
        shared: Arc<Shared>,
    ) -> Self {
        let focus = (mode.parked && mode.focused)
            .then(|| chats.first().copied())
            .flatten();
        Self {
            identity,
            route,
            client,
            chats,
            parked: mode.parked,
            focus,
            shared,
            state: Mutex::new(State::default()),
            recovered: AtomicBool::new(false),
        }
    }

    pub async fn run(self: Arc<Self>, stop: CancellationToken) {
        let stats = &self.shared.stats;
        stats.online(self.route, 1);
        tokio::select! {
            () = stop.cancelled() => {}
            _ = async { tokio::join!(self.sync_loop(), self.act_loop()) } => {}
        }
        stats.online(self.route, -1);
    }

    fn is_parked(&self) -> bool {
        self.parked || self.shared.all_parked.load(Ordering::Relaxed)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        query: &str,
        body: Option<&Value>,
        timeout: Duration,
        timed: bool,
    ) -> Result<Reply, CallError> {
        let stats = &self.shared.stats;
        stats.requested(self.route);
        let started = Instant::now();
        let mut result = self.client.call(method, path, query, body, timeout).await;
        if matches!(&result, Err(error) if error.is_clock()) {
            result = self.client.call(method, path, query, body, timeout).await;
        }
        match &result {
            Ok(_) => {
                let ms = started.elapsed().as_millis() as u64;
                stats.ok(self.route, timed.then_some(ms));
                self.note_recovery();
            }
            Err(error) => stats.error(self.route, &error.label()),
        }
        result
    }

    fn note_recovery(&self) {
        let stats = &self.shared.stats;
        let Some(restart) = stats.restart_ms() else {
            return;
        };
        let now = stats.elapsed_ms();
        if now > restart && !self.recovered.swap(true, Ordering::Relaxed) {
            stats.recovered(now);
        }
    }

    async fn sync_loop(&self) {
        let mut failures: u32 = 0;
        let mut rng = StdRng::from_entropy();
        loop {
            let parked = self.is_parked();
            let hold_ms = match parked {
                true => self
                    .shared
                    .behavior
                    .hold_ms
                    .min(self.client.kind().hold_cap_ms()),
                false => 0,
            };
            let started = Instant::now();
            let started_ms = self.shared.stats.elapsed_ms();
            let body = self.sync_body(hold_ms, parked);
            let gauge = parked.then(|| ParkedGauge::new(&self.shared.stats, self.route));
            let result = self
                .request(
                    "POST",
                    "/chat/sync",
                    "",
                    Some(&body),
                    Duration::from_millis(hold_ms) + HOLD_SLACK,
                    false,
                )
                .await;
            drop(gauge);
            let wait = match result {
                Ok(reply) => {
                    failures = 0;
                    self.absorb(reply, started_ms, parked).await
                }
                Err(error) => {
                    failures += 1;
                    retry_wait(&error, failures, &mut rng)
                }
            };
            let wait = match (parked, failures) {
                (false, 0) => wait.max(
                    self.shared
                        .behavior
                        .short_every
                        .saturating_sub(started.elapsed()),
                ),
                _ => wait,
            };
            if !wait.is_zero() {
                sleep(wait).await;
            }
        }
    }

    fn sync_body(&self, hold_ms: u64, parked: bool) -> Value {
        let mut state = self.state();
        let known: Vec<(Uuid, i64)> = state.known.iter().map(|(id, seq)| (*id, *seq)).collect();
        let marks: Vec<(Uuid, i64)> = state.marks.drain().collect();
        json!({
            "pos": state.pos,
            "inc": state.inc,
            "hold_ms": hold_ms,
            "max_bytes": self.client.kind().answer_bytes(),
            "known": known,
            "focus": self.focus.filter(|_| parked),
            "read": marks,
            "delivered": marks,
        })
    }

    async fn absorb(&self, reply: Reply, started_ms: u64, parked: bool) -> Duration {
        let answer = match reply {
            Reply::Empty { retry_after_ms } => {
                return Duration::from_millis(retry_after_ms.unwrap_or(0));
            }
            Reply::Json(answer) => answer,
        };
        {
            let mut state = self.state();
            state.pos = answer["pos"].as_u64().unwrap_or(state.pos);
            state.inc = answer["inc"]
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok());
        }
        if answer["reset"].as_bool() == Some(true) {
            self.relist().await;
            return Duration::ZERO;
        }
        for conversation in answer["convs"].as_array().into_iter().flatten() {
            self.absorb_conversation(conversation, started_ms, parked)
                .await;
        }
        Duration::ZERO
    }

    async fn absorb_conversation(&self, conversation: &Value, started_ms: u64, parked: bool) {
        let Some(id) = conversation["id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
        else {
            return;
        };
        let epoch = conversation["key_epoch"].as_i64().unwrap_or(0) as i32;
        let last_seq = conversation["last_seq"].as_i64().unwrap_or(0);
        let messages = conversation["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut newest = None;
        for message in &messages {
            let seq = message["seq"].as_i64().unwrap_or(0);
            newest = newest.max(Some(seq));
            self.delivered(id, message, started_ms, parked).await;
        }
        let mut state = self.state();
        state.epochs.insert(id, epoch);
        let known = state.known.entry(id).or_insert(0);
        match newest {
            Some(seq) => *known = (*known).max(seq),
            None if conversation["more"].as_bool() != Some(true) => *known = (*known).max(last_seq),
            None => {}
        }
        if let Some(seq) = newest {
            state.marks.insert(id, seq);
        }
    }

    async fn delivered(&self, conversation: Uuid, message: &Value, started_ms: u64, parked: bool) {
        if message["class"].as_str() != Some("post") {
            return;
        }
        let Some(sender_device) = message["sender_device"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
        else {
            return;
        };
        if sender_device == self.identity.device {
            return;
        }
        let stats = &self.shared.stats;
        let Some(stamp) = self
            .read_message(conversation, sender_device, message)
            .await
        else {
            stats.unreadable(self.route);
            return;
        };
        if stamp.run != self.shared.behavior.run_id {
            return;
        }
        let latency = stats.elapsed_ms().saturating_sub(stamp.sent_ms);
        match parked && started_ms <= stamp.sent_ms {
            true => stats.wake(self.route, latency),
            false => stats.catchup(self.route, latency),
        }
    }

    async fn read_message(
        &self,
        conversation: Uuid,
        sender_device: Uuid,
        message: &Value,
    ) -> Option<Stamp> {
        let epoch = message["epoch"].as_i64()? as i32;
        let client_id = Uuid::parse_str(message["client_id"].as_str()?).ok()?;
        let body = b64::decode(message["body"].as_str()?)?;
        let ck = self.key(conversation, epoch).await.ok()?;
        let sealing = Sealing {
            ck: &ck,
            conversation,
            epoch,
            sender_device,
            client_id,
        };
        content::read_stamp(&content::open(&sealing, &body).ok()?)
    }

    async fn relist(&self) {
        let Ok(Reply::Json(page)) = self
            .request("GET", "/chat/conversations", "", None, CALL_TIMEOUT, true)
            .await
        else {
            return;
        };
        let mut state = self.state();
        for item in page["items"].as_array().into_iter().flatten() {
            let Some(id) = item["id"].as_str().and_then(|id| Uuid::parse_str(id).ok()) else {
                continue;
            };
            let last_seq = item["last_seq"].as_i64().unwrap_or(0);
            let known = state.known.entry(id).or_insert(0);
            *known = (*known).max(last_seq);
            state
                .epochs
                .insert(id, item["key_epoch"].as_i64().unwrap_or(0) as i32);
        }
    }

    async fn key(&self, conversation: Uuid, epoch: i32) -> Result<[u8; 32], CallError> {
        if let Some(key) = self.state().keys.get(&(conversation, epoch)) {
            return Ok(*key);
        }
        let reply = self
            .request(
                "GET",
                &format!("/chat/conversations/{conversation}/keys"),
                &format!("from_epoch={epoch}"),
                None,
                CALL_TIMEOUT,
                true,
            )
            .await?;
        let Reply::Json(keys) = reply else {
            return Err(CallError::Broken("keys answered empty".to_owned()));
        };
        let mut state = self.state();
        for sealed in keys.as_array().into_iter().flatten() {
            let Some(at) = sealed["epoch"].as_i64() else {
                continue;
            };
            let Some(bytes) = sealed["sealed"].as_str().and_then(b64::decode) else {
                continue;
            };
            if let Ok(ck) = chat_wire::ck::open_for_device(
                &self.identity.box_secret,
                conversation.as_bytes(),
                at as u32,
                self.identity.device.as_bytes(),
                &bytes,
            ) {
                state.keys.insert((conversation, at as i32), ck);
                let newest = state.epochs.entry(conversation).or_insert(0);
                *newest = (*newest).max(at as i32);
            }
        }
        state
            .keys
            .get(&(conversation, epoch))
            .copied()
            .ok_or_else(|| CallError::Broken(format!("no key for epoch {epoch}")))
    }

    async fn act_loop(&self) {
        let behavior = &self.shared.behavior;
        if self.chats.is_empty() || (behavior.send_every.is_none() && behavior.type_every.is_none())
        {
            std::future::pending::<()>().await;
        }
        let mut rng = StdRng::from_entropy();
        let start = Instant::now();
        let first =
            |every: Duration, rng: &mut StdRng| start + every.mul_f64(rng.gen_range(0.0..1.0));
        let mut next_send = behavior.send_every.map(|every| first(every, &mut rng));
        let mut next_type = behavior.type_every.map(|every| first(every, &mut rng));
        loop {
            let Some(next) = [next_send, next_type].into_iter().flatten().min() else {
                return;
            };
            sleep_until(next).await;
            let conversation = self.chats[rng.gen_range(0..self.chats.len())];
            let now = Instant::now();
            if let (Some(at), Some(every)) = (next_type, behavior.type_every)
                && at <= now
            {
                self.typing(conversation).await;
                next_type = Some(at + every);
            }
            if let (Some(at), Some(every)) = (next_send, behavior.send_every)
                && at <= now
            {
                self.send(conversation, &mut rng).await;
                next_send = Some(at + every);
            }
        }
    }

    async fn typing(&self, conversation: Uuid) {
        let path = format!("/chat/conversations/{conversation}/typing");
        self.request("POST", &path, "", None, CALL_TIMEOUT, true)
            .await
            .ok();
    }

    async fn send(&self, conversation: Uuid, rng: &mut StdRng) {
        let client_id = Uuid::now_v7();
        let stamp = Stamp {
            run: self.shared.behavior.run_id,
            sender: self.identity.index,
            sent_ms: self.shared.stats.elapsed_ms(),
        };
        let text = content::text(stamp, rng);
        for _ in 0..2 {
            let epoch = self.state().epochs.get(&conversation).copied().unwrap_or(0);
            let Ok(ck) = self.key(conversation, epoch).await else {
                return;
            };
            let sealing = Sealing {
                ck: &ck,
                conversation,
                epoch,
                sender_device: self.identity.device,
                client_id,
            };
            let Ok(body) = content::seal(&sealing, text.clone()) else {
                return;
            };
            let message = json!({
                "client_id": client_id,
                "class": "post",
                "epoch": epoch,
                "body": b64::encode(&body),
            });
            let path = format!("/chat/conversations/{conversation}/messages");
            match self
                .request("POST", &path, "", Some(&message), CALL_TIMEOUT, true)
                .await
            {
                Ok(_) => {
                    self.shared.stats.sent(self.route);
                    return;
                }
                Err(CallError::Inner { code, .. }) if code == "chat_epoch_stale" => {
                    let next = epoch + 1;
                    if self.key(conversation, next).await.is_err() {
                        return;
                    }
                    self.state().epochs.insert(conversation, next);
                }
                Err(_) => return,
            }
        }
    }
}

pub fn retry_wait(error: &CallError, failures: u32, rng: &mut impl Rng) -> Duration {
    if error.is_clock() {
        return Duration::ZERO;
    }
    if let Some(hint) = error.wait_hint() {
        return hint;
    }
    if matches!(error, CallError::Outer { status: 503, .. }) {
        return Duration::from_millis(rng.gen_range(0..=RECONNECT_JITTER_MS));
    }
    let base = 1u64 << failures.saturating_sub(1).min(4);
    let seconds = base.min(BACKOFF_MAX_S) as f64 * rng.gen_range(0.5..1.5);
    Duration::from_secs_f64(seconds)
}

#[cfg(test)]
#[path = "tests/device_tests.rs"]
mod tests;
