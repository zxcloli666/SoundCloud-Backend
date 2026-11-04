use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::remote::{
    CONFIRM_HOLD_SECS, CommandKind, LinkProvider, RemoteCommand, RemoteReply, ReplyEnvelope,
    WaitResponse,
};
use futures::StreamExt;
use mini_moka::sync::Cache;
use tokio::sync::OnceCell;
use tracing::warn;
use uuid::Uuid;

use super::devices::{DEVICE_TTL_MS, Devices};
use super::store::Store;
use super::waits::Waits;
use crate::bus::nats::NatsService;

const PUBLISH_BUDGET: Duration = Duration::from_secs(1);
const EXPIRY_MARGIN_MS: i64 = 1000;
const READ_LIMIT: usize = 32;
pub const PRESENCE_REFRESH: Duration = Duration::from_millis(DEVICE_TTL_MS as u64 / 2);
const PRESENCE_CAPACITY: u64 = 100_000;

pub enum Asked {
    Reply(RemoteReply),
    Offline,
    Silent,
}

#[derive(Debug)]
pub enum Refused {
    TooManyDevices,
    Unavailable,
}

struct Draft<'a> {
    kind: CommandKind,
    source: &'a str,
    text: &'a str,
    hint: Option<String>,
}

pub struct RemoteHub {
    nats: Option<Arc<NatsService>>,
    prefix: String,
    store: OnceCell<Store>,
    waits: Arc<Waits>,
    published: Cache<(String, String), (i64, Instant)>,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn subject_safe(user: &str) -> bool {
    !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl RemoteHub {
    pub fn new(nats: Option<Arc<NatsService>>, prefix: &str) -> Arc<Self> {
        Arc::new(Self {
            nats,
            prefix: prefix.to_owned(),
            store: OnceCell::new(),
            waits: Arc::new(Waits::default()),
            published: Cache::builder()
                .max_capacity(PRESENCE_CAPACITY)
                .time_to_live(Duration::from_millis(DEVICE_TTL_MS as u64))
                .build(),
        })
    }

    pub(super) async fn store(&self) -> Option<&Store> {
        let nats = self.nats.as_ref()?;
        let opened = self
            .store
            .get_or_try_init(|| Store::open(nats.clone(), &self.prefix))
            .await;
        match opened {
            Ok(store) => Some(store),
            Err(error) => {
                warn!(%error, "remote store is unavailable");
                None
            }
        }
    }

    pub async fn prepare(&self) -> bool {
        self.store().await.is_some()
    }

    async fn reachable(&self, user: &str) -> Option<&Store> {
        if !subject_safe(user) {
            return None;
        }
        let store = self.store().await?;
        match store.devices(user).await {
            Ok(devices) if devices.winner(now_ms()).is_some() => Some(store),
            Ok(_) => None,
            Err(error) => {
                warn!(%error, "remote devices were not read");
                None
            }
        }
    }

    fn command(&self, user: &str, draft: Draft<'_>, deadline: Instant) -> RemoteCommand {
        let created_at = now_ms();
        let budget = deadline.saturating_duration_since(Instant::now());
        RemoteCommand {
            id: Uuid::now_v7(),
            kind: draft.kind,
            seq: 0,
            user: user.to_owned(),
            source: draft.source.to_owned(),
            text: draft.text.to_owned(),
            hint: draft.hint,
            created_at,
            expires_at: created_at + budget.as_millis() as i64 + EXPIRY_MARGIN_MS,
        }
    }

    async fn publish(&self, store: &Store, command: &RemoteCommand) -> bool {
        match tokio::time::timeout(PUBLISH_BUDGET, store.publish(command)).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                warn!(%error, "remote command was not published");
                false
            }
            Err(_) => {
                warn!("remote command publish timed out");
                false
            }
        }
    }

    pub async fn dispatch(
        &self,
        user: &str,
        kind: CommandKind,
        source: &str,
        text: &str,
        deadline: Instant,
    ) -> Asked {
        let Some(store) = self.reachable(user).await else {
            return Asked::Offline;
        };
        let draft = Draft {
            kind,
            source,
            text,
            hint: None,
        };
        let command = self.command(user, draft, deadline);
        let mut replies = match store.replies_to(command.id).await {
            Ok(replies) => replies,
            Err(error) => {
                warn!(%error, "remote reply subscription failed");
                return Asked::Silent;
            }
        };
        if !self.publish(store, &command).await {
            return Asked::Silent;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let heard = tokio::time::timeout(remaining, async {
            while let Some(message) = replies.next().await {
                if let Ok(envelope) = serde_json::from_slice::<ReplyEnvelope>(&message.payload)
                    && envelope.user == user
                    && envelope.reply.id == command.id
                {
                    return Some(envelope.reply);
                }
            }
            None
        })
        .await;
        match heard {
            Ok(Some(reply)) => Asked::Reply(reply),
            _ => match self.answer(user, command.id).await {
                Some(reply) => Asked::Reply(reply),
                None => Asked::Silent,
            },
        }
    }

    pub async fn link_confirm(
        &self,
        user: &str,
        provider: LinkProvider,
        hint: &str,
    ) -> Option<RemoteCommand> {
        self.reachable(user).await?;
        let draft = Draft {
            kind: CommandKind::LinkConfirm,
            source: provider.as_str(),
            text: "",
            hint: Some(hint.to_owned()),
        };
        let deadline = Instant::now() + Duration::from_secs(CONFIRM_HOLD_SECS);
        Some(self.command(user, draft, deadline))
    }

    pub async fn post(&self, command: &RemoteCommand) -> bool {
        match self.store().await {
            Some(store) => self.publish(store, command).await,
            None => false,
        }
    }

    pub async fn answer(&self, user: &str, id: Uuid) -> Option<RemoteReply> {
        if !subject_safe(user) {
            return None;
        }
        let store = self.store().await?;
        match store.reply_for(user, id).await {
            Ok(found) => found.map(|envelope| envelope.reply),
            Err(error) => {
                warn!(%error, "remote answer was not read");
                None
            }
        }
    }

    pub async fn reply(&self, user: &str, reply: RemoteReply) -> bool {
        if !subject_safe(user) {
            return false;
        }
        let Some(store) = self.store().await else {
            return false;
        };
        let envelope = ReplyEnvelope {
            user: user.to_owned(),
            reply,
        };
        match store.record_reply(&envelope).await {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "remote answer was not recorded");
                false
            }
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

    async fn announce(&self, store: &Store, user: &str, device: &str, active: i64) {
        if !self.presence_due(user, device, active, Instant::now()) {
            return;
        }
        if let Err(error) = store.note_device(user, device, active, now_ms()).await {
            warn!(%error, "remote presence was not written");
            self.published
                .invalidate(&(user.to_owned(), device.to_owned()));
        }
    }

    pub async fn active_device(&self, user: &str) -> Option<String> {
        if !subject_safe(user) {
            return None;
        }
        let store = self.store().await?;
        let devices = store.devices(user).await.ok()?;
        devices.winner(now_ms()).map(str::to_owned)
    }

    async fn answered(&self, store: &Store, command: &RemoteCommand) -> bool {
        match store.reply_for(&command.user, command.id).await {
            Ok(found) => found.is_some(),
            Err(error) => {
                warn!(%error, "remote answer was not read");
                false
            }
        }
    }

    async fn ready(
        &self,
        store: &Store,
        user: &str,
        device: &str,
        since: u64,
    ) -> (Vec<RemoteCommand>, Devices) {
        let devices = match store.devices(user).await {
            Ok(devices) => devices,
            Err(error) => {
                warn!(%error, "remote devices were not read");
                return (Vec::new(), Devices::default());
            }
        };
        let now = now_ms();
        let mut ready = Vec::new();
        let mut seq = since;
        for _ in 0..READ_LIMIT {
            let stored = match store.next_after(user, seq).await {
                Ok(Some(stored)) => stored,
                Ok(None) => break,
                Err(error) => {
                    warn!(%error, "remote commands were not read");
                    break;
                }
            };
            seq = stored.seq;
            let Some(command) = stored.command else {
                continue;
            };
            if command.expires_at > now
                && devices.is_target(device, now)
                && !self.answered(store, &command).await
            {
                ready.push(command);
            }
        }
        (ready, devices)
    }

    pub async fn wait(
        &self,
        user: &str,
        device: &str,
        active: i64,
        since: Option<i64>,
        hold: Duration,
    ) -> Result<WaitResponse, Refused> {
        if !subject_safe(user) {
            return Err(Refused::Unavailable);
        }
        let store = self.store().await.ok_or(Refused::Unavailable)?;
        let slot = match since {
            Some(_) => Some(
                self.waits
                    .enter(user, device)
                    .ok_or(Refused::TooManyDevices)?,
            ),
            None => None,
        };
        self.announce(store, user, device, active).await;
        let last = match store.last_seq().await {
            Ok(last) => last,
            Err(error) => {
                warn!(%error, "remote cursor was not read");
                return Err(Refused::Unavailable);
            }
        };
        let since = since.map(|since| u64::try_from(since).unwrap_or(0));
        let (Some(since), Some(slot)) = (since.filter(|since| *since <= last), slot) else {
            return Ok(WaitResponse {
                cursor: last as i64,
                commands: Vec::new(),
                active_device: self.active_device(user).await,
            });
        };
        let mut wake = match store.wake(user).await {
            Ok(wake) => wake,
            Err(error) => {
                warn!(%error, "remote wake subscription failed");
                return Err(Refused::Unavailable);
            }
        };
        let deadline = tokio::time::Instant::now() + hold;
        let (mut ready, mut devices) = self.ready(store, user, device, since).await;
        while ready.is_empty() {
            tokio::select! {
                woke = wake.next() => match woke {
                    Some(_) => (ready, devices) = self.ready(store, user, device, since).await,
                    None => break,
                },
                () = slot.replaced.notified() => break,
                () = tokio::time::sleep_until(deadline) => break,
            }
        }
        Ok(WaitResponse {
            cursor: ready.last().map_or(since as i64, |command| command.seq),
            commands: ready,
            active_device: devices.winner(now_ms()).map(str::to_owned),
        })
    }
}
