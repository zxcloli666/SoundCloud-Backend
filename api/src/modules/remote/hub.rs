use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use backend_contracts::remote::{
    COMMAND_SUBJECT, PRESENCE_SUBJECT, Presence, RemoteCommand, RemoteReply, ReplyEnvelope,
    WaitResponse, reply_subject, reply_subject_filter,
};
use bytes::Bytes;
use deadpool_redis::Pool;
use futures::StreamExt;
use mini_moka::sync::Cache;
use tokio::sync::{broadcast, oneshot};
use tracing::warn;
use uuid::Uuid;

use super::buffer::Buffer;
use super::roster::Roster;
use crate::bus::nats::NatsService;

const CHANNEL_CAPACITY: usize = 256;
const PUBLISH_BUDGET: Duration = Duration::from_millis(250);
const EXPIRY_MARGIN_MS: i64 = 1000;

pub enum Asked {
    Reply(RemoteReply),
    Offline,
    Silent,
}

struct Pending {
    user: String,
    reply: oneshot::Sender<RemoteReply>,
}

pub struct RemoteHub {
    nats: Option<Arc<NatsService>>,
    buffer: Buffer,
    roster: Roster,
    commands: broadcast::Sender<Arc<RemoteCommand>>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    seen: Cache<Uuid, ()>,
    last_seq: AtomicI64,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn now_micros() -> i64 {
    chrono::Utc::now().timestamp_micros()
}

impl RemoteHub {
    pub fn new(nats: Option<Arc<NatsService>>, redis: Pool, prefix: &str) -> Arc<Self> {
        let (commands, _) = broadcast::channel(CHANNEL_CAPACITY);
        Arc::new(Self {
            nats,
            buffer: Buffer::new(redis, prefix),
            roster: Roster::default(),
            commands,
            pending: Mutex::new(HashMap::new()),
            seen: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(180))
                .build(),
            last_seq: AtomicI64::new(0),
        })
    }

    pub async fn start(self: &Arc<Self>) {
        let Some(nats) = &self.nats else { return };
        match nats.subscribe(COMMAND_SUBJECT).await {
            Ok(mut stream) => {
                let hub = self.clone();
                tokio::spawn(async move {
                    while let Some(message) = stream.next().await {
                        if let Ok(command) = serde_json::from_slice(&message.payload) {
                            hub.receive(command).await;
                        }
                    }
                });
            }
            Err(error) => warn!(%error, "remote commands are local only"),
        }
        match nats.subscribe(PRESENCE_SUBJECT).await {
            Ok(mut stream) => {
                let hub = self.clone();
                tokio::spawn(async move {
                    while let Some(message) = stream.next().await {
                        if let Ok(seen) = serde_json::from_slice::<Presence>(&message.payload) {
                            hub.roster
                                .note(&seen.user, &seen.device, seen.active, Instant::now());
                        }
                    }
                });
            }
            Err(error) => warn!(%error, "remote presence is local only"),
        }
        match nats.subscribe(&reply_subject_filter()).await {
            Ok(mut stream) => {
                let hub = self.clone();
                tokio::spawn(async move {
                    while let Some(message) = stream.next().await {
                        if let Ok(envelope) = serde_json::from_slice(&message.payload) {
                            hub.deliver_reply(envelope);
                        }
                    }
                });
            }
            Err(error) => warn!(%error, "remote replies are local only"),
        }
    }

    fn next_seq(&self) -> i64 {
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

    async fn publish(&self, subject: String, payload: Vec<u8>) {
        let Some(nats) = &self.nats else { return };
        let publish = nats.publish_core(&subject, Bytes::from(payload));
        match tokio::time::timeout(PUBLISH_BUDGET, publish).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, subject, "remote fan-out failed"),
            Err(_) => warn!(subject, "remote fan-out timed out"),
        }
    }

    async fn receive(&self, command: RemoteCommand) {
        if self.seen.contains_key(&command.id) {
            return;
        }
        self.seen.insert(command.id, ());
        if self.roster.polled_recently(&command.user, Instant::now())
            && let Err(error) = self.buffer.push(&command).await
        {
            warn!(%error, "remote command was not buffered");
        }
        let _ = self.commands.send(Arc::new(command));
    }

    pub async fn dispatch(&self, user: &str, source: &str, text: &str, wait: Duration) -> Asked {
        if self.roster.winner(user, Instant::now()).is_none() {
            return Asked::Offline;
        }
        let created_at = now_ms();
        let command = RemoteCommand {
            id: Uuid::now_v7(),
            seq: self.next_seq(),
            user: user.to_owned(),
            source: source.to_owned(),
            text: text.to_owned(),
            created_at,
            expires_at: created_at + wait.as_millis() as i64 + EXPIRY_MARGIN_MS,
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
        if let Ok(payload) = serde_json::to_vec(&command) {
            self.publish(COMMAND_SUBJECT.to_owned(), payload).await;
        }
        let id = command.id;
        self.receive(command).await;
        match tokio::time::timeout(wait, rx).await {
            Ok(Ok(reply)) => Asked::Reply(reply),
            _ => {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Asked::Silent
            }
        }
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
        self.roster.note(user, device, active, Instant::now());
        let presence = Presence {
            user: user.to_owned(),
            device: device.to_owned(),
            active,
        };
        if let Ok(payload) = serde_json::to_vec(&presence) {
            self.publish(PRESENCE_SUBJECT.to_owned(), payload).await;
        }
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
    ) -> WaitResponse {
        self.roster.mark_polled(user, Instant::now());
        self.announce(user, device, active).await;
        let Some(since) = since else {
            return WaitResponse {
                cursor: now_micros(),
                commands: Vec::new(),
                active_device: self.active_device(user),
            };
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
        WaitResponse {
            cursor: ready.last().map_or(since, |command| command.seq),
            commands: ready,
            active_device: self.active_device(user),
        }
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
