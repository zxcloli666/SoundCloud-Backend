use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use backend_contracts::remote::{
    CONFIRM_HOLD_SECS, CommandKind, LinkProvider, RemoteCommand, RemoteReply, ReplyEnvelope,
    WaitResponse,
};
use futures::StreamExt;
use mini_moka::sync::Cache;
use tracing::warn;
use uuid::{Uuid, Version};

use super::devices::{DEVICE_TTL_MS, Devices};
use super::store::{COMMANDS_AGE, SCAN_LIMIT, Store, StoreResult};
use super::waits::Waits;
use crate::bus::nats::NatsService;

const PUBLISH_BUDGET: Duration = Duration::from_secs(1);
const ANNOUNCE_BUDGET: Duration = Duration::from_secs(1);
const LATE_ANSWER_BUDGET: Duration = Duration::from_millis(250);
const REOPEN_AFTER: Duration = Duration::from_secs(5);
const RECHECK_AFTER: Duration = Duration::from_millis(50);
const EXPIRY_MARGIN_MS: i64 = 1000;
const REPLY_WINDOW_MARGIN_MS: i64 = 30_000;
const REPLY_CLOCK_SKEW_MS: i64 = 5_000;
pub const PRESENCE_REFRESH: Duration = Duration::from_millis(DEVICE_TTL_MS as u64 / 2);
const PRESENCE_CAPACITY: u64 = 100_000;

pub enum Asked {
    Reply(RemoteReply),
    Offline,
    Silent,
    Busy,
}

#[derive(Debug)]
pub enum Refused {
    TooManyDevices,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreached {
    Offline,
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replied {
    Recorded,
    Unknown,
    Busy,
}

enum Published {
    Sent,
    Unsure,
    Failed,
}

struct Draft<'a> {
    kind: CommandKind,
    source: &'a str,
    text: &'a str,
    hint: Option<String>,
}

#[derive(Default)]
struct Slot {
    store: Option<Arc<Store>>,
    failed_at: Option<Instant>,
}

enum Cached {
    Open(Arc<Store>),
    Failing,
    Unknown,
}

struct Scan {
    from: u64,
    ready: Vec<RemoteCommand>,
    devices: Devices,
}

pub struct RemoteHub {
    nats: Option<Arc<NatsService>>,
    prefix: String,
    slot: Mutex<Slot>,
    opening: tokio::sync::Mutex<()>,
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

pub fn fresh_command_id(id: Uuid, now_ms: i64) -> bool {
    if id.get_version() != Some(Version::SortRand) {
        return false;
    }
    let Some(timestamp) = id.get_timestamp() else {
        return false;
    };
    let (seconds, nanos) = timestamp.to_unix();
    let created_ms = seconds as i64 * 1000 + i64::from(nanos / 1_000_000);
    let window_ms = COMMANDS_AGE.as_millis() as i64 + REPLY_WINDOW_MARGIN_MS;
    created_ms <= now_ms + REPLY_CLOCK_SKEW_MS && now_ms - created_ms <= window_ms
}

impl RemoteHub {
    pub fn new(nats: Option<Arc<NatsService>>, prefix: &str) -> Arc<Self> {
        Arc::new(Self {
            nats,
            prefix: prefix.to_owned(),
            slot: Mutex::new(Slot::default()),
            opening: tokio::sync::Mutex::new(()),
            waits: Arc::new(Waits::default()),
            published: Cache::builder()
                .max_capacity(PRESENCE_CAPACITY)
                .time_to_live(Duration::from_millis(DEVICE_TTL_MS as u64))
                .build(),
        })
    }

    fn cached(&self) -> Cached {
        let slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = &slot.store {
            return Cached::Open(store.clone());
        }
        match slot.failed_at {
            Some(at) if at.elapsed() < REOPEN_AFTER => Cached::Failing,
            _ => Cached::Unknown,
        }
    }

    pub(super) async fn store(&self) -> Option<Arc<Store>> {
        let nats = self.nats.as_ref()?;
        if nats.disconnected() {
            return None;
        }
        match self.cached() {
            Cached::Open(store) => return Some(store),
            Cached::Failing => return None,
            Cached::Unknown => {}
        }
        let _opening = self.opening.lock().await;
        match self.cached() {
            Cached::Open(store) => return Some(store),
            Cached::Failing => return None,
            Cached::Unknown => {}
        }
        let opened = Store::open(nats, &self.prefix).await;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        match opened {
            Ok(store) => {
                let store = Arc::new(store);
                slot.store = Some(store.clone());
                slot.failed_at = None;
                Some(store)
            }
            Err(error) => {
                warn!(%error, "remote store is unavailable");
                slot.failed_at = Some(Instant::now());
                None
            }
        }
    }

    fn forget(&self, store: &Arc<Store>) {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .store
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, store))
        {
            slot.store = None;
            self.published.invalidate_all();
        }
    }

    fn checked<T>(&self, store: &Arc<Store>, what: &str, result: StoreResult<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                warn!(%error, "remote {what} failed");
                if error.missing {
                    self.forget(store);
                }
                None
            }
        }
    }

    pub async fn prepare(&self) -> bool {
        self.store().await.is_some()
    }

    async fn reachable(&self, user: &str) -> Result<Arc<Store>, Unreached> {
        if !subject_safe(user) {
            return Err(Unreached::Offline);
        }
        let store = self.store().await.ok_or(Unreached::Busy)?;
        let devices = self
            .checked(&store, "devices read", store.devices(user).await)
            .ok_or(Unreached::Busy)?;
        match devices.winner(now_ms()) {
            Some(_) => Ok(store),
            None => Err(Unreached::Offline),
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

    async fn publish(&self, store: &Arc<Store>, command: &RemoteCommand) -> Published {
        match tokio::time::timeout(PUBLISH_BUDGET, store.publish(command)).await {
            Ok(result) => match self.checked(store, "command publish", result) {
                Some(()) => Published::Sent,
                None => Published::Failed,
            },
            Err(_) => {
                warn!("remote command publish timed out, the reply may still come");
                Published::Unsure
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
        let store = match self.reachable(user).await {
            Ok(store) => store,
            Err(Unreached::Offline) => return Asked::Offline,
            Err(Unreached::Busy) => return Asked::Busy,
        };
        let draft = Draft {
            kind,
            source,
            text,
            hint: None,
        };
        let command = self.command(user, draft, deadline);
        let Some(mut replies) = self.checked(
            &store,
            "reply subscription",
            store.replies_to(command.id).await,
        ) else {
            return Asked::Busy;
        };
        if let Published::Failed = self.publish(&store, &command).await {
            return Asked::Busy;
        }
        let heard = tokio::time::timeout_at(deadline.into(), async {
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
        if let Ok(Some(reply)) = heard {
            return Asked::Reply(reply);
        }
        match tokio::time::timeout(LATE_ANSWER_BUDGET, self.answer(user, command.id)).await {
            Ok(Some(reply)) => Asked::Reply(reply),
            _ => Asked::Silent,
        }
    }

    pub async fn link_confirm(
        &self,
        user: &str,
        provider: LinkProvider,
        hint: &str,
    ) -> Result<RemoteCommand, Unreached> {
        self.reachable(user).await?;
        let draft = Draft {
            kind: CommandKind::LinkConfirm,
            source: provider.as_str(),
            text: "",
            hint: Some(hint.to_owned()),
        };
        let deadline = Instant::now() + Duration::from_secs(CONFIRM_HOLD_SECS);
        Ok(self.command(user, draft, deadline))
    }

    pub async fn post(&self, command: &RemoteCommand) -> bool {
        let Some(store) = self.store().await else {
            return false;
        };
        !matches!(self.publish(&store, command).await, Published::Failed)
    }

    pub async fn answer(&self, user: &str, id: Uuid) -> Option<RemoteReply> {
        if !subject_safe(user) {
            return None;
        }
        let store = self.store().await?;
        self.checked(&store, "answer read", store.reply_for(user, id).await)
            .flatten()
            .map(|envelope| envelope.reply)
    }

    pub async fn reply(&self, user: &str, reply: RemoteReply) -> Replied {
        if !subject_safe(user) || !fresh_command_id(reply.id, now_ms()) {
            return Replied::Unknown;
        }
        let Some(store) = self.store().await else {
            return Replied::Busy;
        };
        let envelope = ReplyEnvelope {
            user: user.to_owned(),
            reply,
        };
        match self.checked(&store, "answer write", store.record_reply(&envelope).await) {
            Some(()) => Replied::Recorded,
            None => Replied::Busy,
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

    async fn announce(&self, store: &Arc<Store>, user: &str, device: &str, active: i64) {
        if !self.presence_due(user, device, active, Instant::now()) {
            return;
        }
        let written = tokio::time::timeout(
            ANNOUNCE_BUDGET,
            store.note_device(user, device, active, now_ms()),
        )
        .await;
        let noted = match written {
            Ok(result) => self.checked(store, "presence write", result).is_some(),
            Err(_) => {
                warn!("remote presence write timed out");
                false
            }
        };
        if !noted {
            self.published
                .invalidate(&(user.to_owned(), device.to_owned()));
        }
    }

    #[cfg(test)]
    pub fn held(&self, user: &str) -> usize {
        self.waits.count(user)
    }

    pub async fn active_device(&self, user: &str) -> Option<String> {
        if !subject_safe(user) {
            return None;
        }
        let store = self.store().await?;
        let devices = self.checked(&store, "devices read", store.devices(user).await)?;
        devices.winner(now_ms()).map(str::to_owned)
    }

    async fn deliverable(&self, store: &Arc<Store>, command: &RemoteCommand, device: &str) -> bool {
        let answered = self.checked(
            store,
            "answer read",
            store.reply_for(&command.user, command.id).await,
        );
        if matches!(answered, Some(Some(_))) {
            return false;
        }
        self.checked(
            store,
            "command claim",
            store.claim(&command.user, command.id, device).await,
        )
        .unwrap_or(false)
    }

    async fn scan_once(&self, store: &Arc<Store>, user: &str, device: &str, scan: &mut Scan) {
        let Some(devices) = self.checked(store, "devices read", store.devices(user).await) else {
            return;
        };
        scan.devices = devices;
        let now = now_ms();
        for _ in 0..SCAN_LIMIT {
            let read = store.next_after(user, scan.from).await;
            let Some(Some(stored)) = self.checked(store, "command read", read) else {
                return;
            };
            if let Some(command) = stored.command
                && command.expires_at > now
                && scan.devices.is_target(device, now)
                && self.deliverable(store, &command, device).await
            {
                scan.ready.push(command);
            }
            scan.from = stored.seq;
        }
    }

    async fn scan(
        &self,
        store: &Arc<Store>,
        user: &str,
        device: &str,
        scan: &mut Scan,
        deadline: tokio::time::Instant,
    ) {
        let _ = tokio::time::timeout_at(deadline, self.scan_once(store, user, device, scan)).await;
    }

    pub async fn wait(
        &self,
        user: &str,
        device: &str,
        active: i64,
        since: Option<i64>,
        hold: Duration,
    ) -> Result<WaitResponse, Refused> {
        let deadline = tokio::time::Instant::now() + hold;
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
        self.announce(&store, user, device, active).await;
        let last = self
            .checked(&store, "cursor read", store.last_seq().await)
            .ok_or(Refused::Unavailable)?;
        let since = since.map(|since| u64::try_from(since).unwrap_or(0));
        let (Some(since), Some(slot)) = (since.filter(|since| *since <= last), slot) else {
            return Ok(WaitResponse {
                cursor: last as i64,
                commands: Vec::new(),
                active_device: self.active_device(user).await,
            });
        };
        let mut wake = self
            .checked(&store, "wake subscription", store.wake(user).await)
            .ok_or(Refused::Unavailable)?;
        let mut scan = Scan {
            from: since,
            ready: Vec::new(),
            devices: Devices::default(),
        };
        self.scan(&store, user, device, &mut scan, deadline).await;
        while scan.ready.is_empty() {
            tokio::select! {
                woke = wake.next() => match woke {
                    Some(_) => {
                        self.scan(&store, user, device, &mut scan, deadline).await;
                        if scan.ready.is_empty() {
                            let recheck = tokio::time::Instant::now() + RECHECK_AFTER;
                            tokio::time::sleep_until(recheck.min(deadline)).await;
                            self.scan(&store, user, device, &mut scan, deadline).await;
                        }
                    }
                    None => break,
                },
                () = slot.replaced.notified() => break,
                () = tokio::time::sleep_until(deadline) => break,
            }
        }
        Ok(WaitResponse {
            cursor: scan.from as i64,
            commands: scan.ready,
            active_device: scan.devices.winner(now_ms()).map(str::to_owned),
        })
    }
}
