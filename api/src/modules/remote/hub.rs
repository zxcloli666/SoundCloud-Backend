use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use backend_contracts::remote::{
    COMMAND_SUBJECT, CONFIRM_HOLD_SECS, CommandKind, LinkProvider, PRESENCE_SUBJECT, Presence,
    RemoteCommand, RemoteReply, ReplyEnvelope, WaitResponse, reply_subject, reply_subject_filter,
};
use bytes::Bytes;
use deadpool_redis::Pool;
use futures::StreamExt;
use mini_moka::sync::Cache;
use tokio::sync::{broadcast, oneshot};
use tracing::warn;
use uuid::Uuid;

use super::buffer::Buffer;
use super::roster::{DEVICE_TTL, Roster};
use super::waits::Waits;
use crate::bus::nats::NatsService;

const CHANNEL_CAPACITY: usize = 256;
const PUBLISH_BUDGET: Duration = Duration::from_millis(250);
const EXPIRY_MARGIN_MS: i64 = 1000;
const RESUBSCRIBE_MIN: Duration = Duration::from_millis(500);
const RESUBSCRIBE_MAX: Duration = Duration::from_secs(30);
pub const PRESENCE_REFRESH: Duration = Duration::from_secs(DEVICE_TTL.as_secs() / 2);
const PRESENCE_CAPACITY: u64 = 100_000;

pub enum Asked {
    Reply(RemoteReply),
    Offline,
    Silent,
}

#[derive(Clone, Copy)]
enum Topic {
    Commands,
    Presence,
    Replies,
}

impl Topic {
    fn subject(self) -> String {
        match self {
            Self::Commands => COMMAND_SUBJECT.to_owned(),
            Self::Presence => PRESENCE_SUBJECT.to_owned(),
            Self::Replies => reply_subject_filter(),
        }
    }
}

struct Pending {
    user: String,
    reply: oneshot::Sender<RemoteReply>,
}

struct Draft<'a> {
    kind: CommandKind,
    source: &'a str,
    text: &'a str,
    hint: Option<String>,
}

struct PendingGuard<'a> {
    pending: &'a Mutex<HashMap<Uuid, Pending>>,
    id: Uuid,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

pub struct RemoteHub {
    nats: Option<Arc<NatsService>>,
    buffer: Buffer,
    roster: Roster,
    waits: Arc<Waits>,
    commands: broadcast::Sender<Arc<RemoteCommand>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    seen: Cache<Uuid, ()>,
    published: Cache<(String, String), (i64, Instant)>,
    last_seq: AtomicI64,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn now_micros() -> i64 {
    chrono::Utc::now().timestamp_micros()
}

impl RemoteHub {
    pub fn new(nats: Option<Arc<NatsService>>, redis: Pool, prefix: &str) -> Arc<Self> {
        let (commands, _) = broadcast::channel(CHANNEL_CAPACITY);
        Arc::new(Self {
            nats,
            buffer: Buffer::new(redis, prefix),
            roster: Roster::default(),
            waits: Arc::new(Waits::default()),
            commands,
            pending: Mutex::new(HashMap::new()),
            seen: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(180))
                .build(),
            published: Cache::builder()
                .max_capacity(PRESENCE_CAPACITY)
                .time_to_live(DEVICE_TTL)
                .build(),
            last_seq: AtomicI64::new(0),
        })
    }

    pub async fn start(self: &Arc<Self>) {
        let Some(nats) = self.nats.clone() else {
            return;
        };
        for topic in [Topic::Commands, Topic::Presence, Topic::Replies] {
            let first = match nats.subscribe(&topic.subject()).await {
                Ok(stream) => Some(stream),
                Err(error) => {
                    warn!(%error, "remote subscription failed, retrying");
                    None
                }
            };
            let hub = self.clone();
            let nats = nats.clone();
            tokio::spawn(async move { hub.follow(nats, topic, first).await });
        }
    }

    async fn follow(
        self: Arc<Self>,
        nats: Arc<NatsService>,
        topic: Topic,
        mut ready: Option<async_nats::Subscriber>,
    ) {
        let mut delay = RESUBSCRIBE_MIN;
        loop {
            let stream = match ready.take() {
                Some(stream) => stream,
                None => match nats.subscribe(&topic.subject()).await {
                    Ok(stream) => stream,
                    Err(error) => {
                        warn!(%error, "remote subscription failed, retrying");
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(RESUBSCRIBE_MAX);
                        continue;
                    }
                },
            };
            delay = RESUBSCRIBE_MIN;
            self.pump(stream, topic).await;
            warn!("remote subscription ended, subscribing again");
            tokio::time::sleep(RESUBSCRIBE_MIN).await;
        }
    }

    async fn pump(&self, mut stream: async_nats::Subscriber, topic: Topic) {
        while let Some(message) = stream.next().await {
            match topic {
                Topic::Commands => {
                    if let Ok(command) = serde_json::from_slice(&message.payload) {
                        self.receive(command).await;
                    }
                }
                Topic::Presence => {
                    if let Ok(seen) = serde_json::from_slice::<Presence>(&message.payload) {
                        self.roster
                            .note(&seen.user, &seen.device, seen.active, Instant::now());
                    }
                }
                Topic::Replies => {
                    if let Ok(envelope) = serde_json::from_slice(&message.payload) {
                        self.deliver_reply(envelope);
                    }
                }
            }
        }
    }

    fn local_seq(&self) -> i64 {
        loop {
            let previous = self.last_seq.load(Ordering::Relaxed);
            let next = now_micros().max(previous + 1);
            if self
                .last_seq
                .compare_exchange(previous, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    async fn next_seq(&self, user: &str) -> i64 {
        match self.buffer.next_seq(user, now_micros()).await {
            Ok(seq) => seq.max(self.local_seq()),
            Err(error) => {
                warn!(%error, "remote sequence fell back to the local clock");
                self.local_seq()
            }
        }
    }

    async fn publish(&self, subject: String, payload: Vec<u8>) {
        let Some(nats) = &self.nats else { return };
        let publish = nats.publish_core(&subject, Bytes::from(payload));
        match tokio::time::timeout(PUBLISH_BUDGET, publish).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, subject, "remote fan-out failed"),
            Err(_) => warn!(subject, "remote fan-out timed out"),
        }
    }

    pub(super) async fn receive(&self, command: RemoteCommand) {
        if self.seen.contains_key(&command.id) {
            return;
        }
        self.seen.insert(command.id, ());
        let command = Arc::new(command);
        let _ = self.commands.send(command.clone());
        if self.roster.polled_recently(&command.user, Instant::now()) {
            let buffer = self.buffer.clone();
            tokio::spawn(async move {
                if let Err(error) = buffer.push(&command).await {
                    warn!(%error, "remote command was not buffered");
                }
            });
        }
    }

    pub async fn confirm_link(&self, user: &str, provider: LinkProvider, hint: &str) -> Asked {
        let deadline = Instant::now() + Duration::from_secs(CONFIRM_HOLD_SECS);
        let draft = Draft {
            kind: CommandKind::LinkConfirm,
            source: provider.as_str(),
            text: "",
            hint: Some(hint.to_owned()),
        };
        self.send(user, draft, deadline).await
    }

    pub async fn dispatch(
        &self,
        user: &str,
        kind: CommandKind,
        source: &str,
        text: &str,
        deadline: Instant,
    ) -> Asked {
        let draft = Draft {
            kind,
            source,
            text,
            hint: None,
        };
        self.send(user, draft, deadline).await
    }

    async fn send(&self, user: &str, draft: Draft<'_>, deadline: Instant) -> Asked {
        if self.roster.winner(user, Instant::now()).is_none() {
            return Asked::Offline;
        }
        let created_at = now_ms();
        let budget = deadline.saturating_duration_since(Instant::now());
        let command = RemoteCommand {
            id: Uuid::now_v7(),
            kind: draft.kind,
            seq: self.next_seq(user).await,
            user: user.to_owned(),
            source: draft.source.to_owned(),
            text: draft.text.to_owned(),
            hint: draft.hint,
            created_at,
            expires_at: created_at + budget.as_millis() as i64 + EXPIRY_MARGIN_MS,
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                command.id,
                Pending {
                    user: user.to_owned(),
                    reply: tx,
                },
            );
        let _cleanup = PendingGuard {
            pending: &self.pending,
            id: command.id,
        };
        if let Ok(payload) = serde_json::to_vec(&command) {
            self.publish(COMMAND_SUBJECT.to_owned(), payload).await;
        }
        self.receive(command).await;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, rx).await {
            Ok(Ok(reply)) => Asked::Reply(reply),
            _ => Asked::Silent,
        }
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub async fn reply(&self, user: &str, reply: RemoteReply) {
        let envelope = ReplyEnvelope {
            user: user.to_owned(),
            reply,
        };
        let subject = reply_subject(envelope.reply.id);
        let payload = serde_json::to_vec(&envelope).unwrap_or_default();
        self.deliver_reply(envelope);
        self.publish(subject, payload).await;
    }

    fn deliver_reply(&self, envelope: ReplyEnvelope) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let owned = pending
            .get(&envelope.reply.id)
            .is_some_and(|waiting| waiting.user == envelope.user);
        if owned && let Some(waiting) = pending.remove(&envelope.reply.id) {
            let _ = waiting.reply.send(envelope.reply);
        }
    }

    pub async fn announce(&self, user: &str, device: &str, active: i64) {
        let now = Instant::now();
        self.roster.note(user, device, active, now);
        if !self.presence_due(user, device, active, now) {
            return;
        }
        let presence = Presence {
            user: user.to_owned(),
            device: device.to_owned(),
            active,
        };
        if let Ok(payload) = serde_json::to_vec(&presence) {
            self.publish(PRESENCE_SUBJECT.to_owned(), payload).await;
        }
    }

    pub(super) fn presence_due(&self, user: &str, device: &str, active: i64, now: Instant) -> bool {
        let key = (user.to_owned(), device.to_owned());
        let due = match self.published.get(&key) {
            Some((sent_active, sent_at)) => {
                sent_active != active || now.duration_since(sent_at) >= PRESENCE_REFRESH
            }
            None => true,
        };
        if due {
            self.published.insert(key, (active, now));
        }
        due
    }

    pub fn active_device(&self, user: &str) -> Option<String> {
        self.roster.winner(user, Instant::now())
    }

    pub async fn wait(
        &self,
        user: &str,
        device: &str,
        active: i64,
        since: Option<i64>,
        hold: Duration,
    ) -> Option<WaitResponse> {
        let slot = match since {
            Some(_) => Some(self.waits.enter(user, device)?),
            None => None,
        };
        self.roster.mark_polled(user, Instant::now());
        self.announce(user, device, active).await;
        let (Some(since), Some(slot)) = (since, slot) else {
            return Some(WaitResponse {
                cursor: now_micros(),
                commands: Vec::new(),
                active_device: self.active_device(user),
            });
        };
        let mut incoming = self.commands.subscribe();
        let deadline = tokio::time::Instant::now() + hold;
        let mut ready = self.buffered(user, device, since).await;
        while ready.is_empty() {
            tokio::select! {
                received = incoming.recv() => match received {
                    Ok(command) => {
                        if command.user == user && command.seq > since && self.deliverable(&command, device) {
                            ready.push((*command).clone());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        ready = self.buffered(user, device, since).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                () = slot.replaced.notified() => {
                    return Some(WaitResponse {
                        cursor: since,
                        commands: Vec::new(),
                        active_device: self.active_device(user),
                    });
                }
                () = tokio::time::sleep_until(deadline) => break,
            }
        }
        while let Ok(command) = incoming.try_recv() {
            if command.user == user
                && command.seq > since
                && self.deliverable(&command, device)
                && !ready.iter().any(|known| known.id == command.id)
            {
                ready.push((*command).clone());
            }
        }
        ready.sort_by_key(|command| command.seq);
        Some(WaitResponse {
            cursor: ready.last().map_or(since, |command| command.seq),
            commands: ready,
            active_device: self.active_device(user),
        })
    }

    fn deliverable(&self, command: &RemoteCommand, device: &str) -> bool {
        command.expires_at > now_ms()
            && self.roster.is_target(&command.user, device, Instant::now())
    }

    async fn buffered(&self, user: &str, device: &str, since: i64) -> Vec<RemoteCommand> {
        match self.buffer.since(user, since).await {
            Ok(commands) => commands
                .into_iter()
                .filter(|command| self.deliverable(command, device))
                .collect(),
            Err(error) => {
                warn!(%error, "remote buffer was not read");
                Vec::new()
            }
        }
    }
}
