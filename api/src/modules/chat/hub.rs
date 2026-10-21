use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{OnceCell, watch};
use tokio::time::Instant;
use uuid::Uuid;

use crate::modules::chat::sync::bucket::Bucket;
use crate::modules::chat::sync::prefs::PrefsCache;
#[cfg(test)]
use crate::modules::chat::sync::shards::SHARDS;
use crate::modules::chat::sync::shards::Shards;
use crate::modules::chat::typing::TypingBoard;

pub const COALESCE: Duration = Duration::from_millis(500);
pub const SEEN_EVERY: Duration = Duration::from_secs(60);
const SLOT_IDLE: Duration = Duration::from_secs(30);
const PRUNE_EVERY: u64 = 256;
const WAKES_COALESCED: &str = "chat_wakes_coalesced_total";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signal {
    pub at: Option<Instant>,
    pub woken: Option<Instant>,
    pub focus: bool,
    pub superseded: bool,
}

impl Signal {
    fn wake(&mut self, at: Instant, now: Instant, focus: bool) {
        self.at = Some(self.at.map_or(at, |earlier| earlier.min(at)));
        self.woken.get_or_insert(now);
        self.focus |= focus;
    }

    fn ended() -> Self {
        Self {
            superseded: true,
            ..Self::default()
        }
    }
}

pub struct ChatHub {
    users: Shards<String, UserSlot>,
    focused: Shards<Uuid, Vec<FocusedPoll>>,
    parked: AtomicUsize,
    next_poll: AtomicU64,
    pub typing: TypingBoard,
    pub prefs: PrefsCache,
    pub incarnation: OnceCell<Uuid>,
}

struct UserSlot {
    polls: Vec<ParkedPoll>,
    devices: Vec<DeviceSlot>,
    fire_at: Option<Instant>,
    seen_at: Option<Instant>,
    touched: Instant,
}

struct ParkedPoll {
    id: u64,
    device: Uuid,
    signal: watch::Sender<Signal>,
}

struct DeviceSlot {
    id: Uuid,
    bucket: Bucket,
    focus: Option<Uuid>,
}

#[derive(Clone)]
struct FocusedPoll {
    user: String,
    id: u64,
}

impl UserSlot {
    fn new(now: Instant) -> Self {
        Self {
            polls: Vec::new(),
            devices: Vec::new(),
            fire_at: None,
            seen_at: None,
            touched: now,
        }
    }

    fn device(&mut self, id: Uuid, now: Instant) -> &mut DeviceSlot {
        let index = match self.devices.iter().position(|device| device.id == id) {
            Some(index) => index,
            None => {
                self.devices.push(DeviceSlot {
                    id,
                    bucket: Bucket::full(now),
                    focus: None,
                });
                self.devices.len() - 1
            }
        };
        &mut self.devices[index]
    }

    fn next_fire(&mut self, now: Instant) -> Instant {
        let at = match self.fire_at {
            Some(fire) if fire >= now => fire,
            Some(fire) if now < fire + COALESCE => fire + COALESCE,
            _ => now,
        };
        if at > now {
            metrics::counter!(WAKES_COALESCED).increment(1);
        }
        self.fire_at = Some(at);
        at
    }

    fn signal(&mut self, now: Instant, focus: bool, wanted: impl Fn(&ParkedPoll) -> bool) {
        if !self.polls.iter().any(&wanted) {
            return;
        }
        let at = self.next_fire(now);
        for poll in self.polls.iter().filter(|poll| wanted(poll)) {
            poll.signal
                .send_modify(|signal| signal.wake(at, now, focus));
        }
    }

    fn idle(&self, now: Instant) -> bool {
        self.polls.is_empty() && now.saturating_duration_since(self.touched) >= SLOT_IDLE
    }
}

impl ChatHub {
    pub fn new() -> Self {
        Self {
            users: Shards::new(),
            focused: Shards::new(),
            parked: AtomicUsize::new(0),
            next_poll: AtomicU64::new(1),
            typing: TypingBoard::new(),
            prefs: PrefsCache::new(),
            incarnation: OnceCell::new(),
        }
    }

    pub fn parked(&self) -> usize {
        self.parked.load(Ordering::Relaxed)
    }

    pub fn park(&self, user_id: &str, device: Uuid, focus: Option<Uuid>) -> Parked<'_> {
        let id = self.next_poll.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = watch::channel(Signal::default());
        let now = Instant::now();
        {
            let mut users = self.users.lock(user_id);
            let slot = users
                .entry(user_id.to_owned())
                .or_insert_with(|| UserSlot::new(now));
            slot.touched = now;
            for older in slot.polls.iter().filter(|poll| poll.device == device) {
                older.signal.send_modify(|signal| signal.superseded = true);
            }
            slot.polls.push(ParkedPoll {
                id,
                device,
                signal: sender,
            });
        }
        if let Some(conversation) = focus {
            self.focused
                .lock(&conversation)
                .entry(conversation)
                .or_default()
                .push(FocusedPoll {
                    user: user_id.to_owned(),
                    id,
                });
        }
        self.parked.fetch_add(1, Ordering::Relaxed);
        if id.is_multiple_of(PRUNE_EVERY) {
            self.prune(now, (id / PRUNE_EVERY) as usize);
        }
        Parked {
            hub: self,
            user_id: user_id.to_owned(),
            id,
            focus,
            signal: receiver,
        }
    }

    pub fn wake(&self, user_id: &str) {
        let now = Instant::now();
        if let Some(slot) = self.users.lock(user_id).get_mut(user_id) {
            slot.signal(now, false, |_| true);
        }
    }

    pub fn wake_focus(&self, conversation: Uuid, except_user: &str) {
        let mut polls = self
            .focused
            .lock(&conversation)
            .get(&conversation)
            .cloned()
            .unwrap_or_default();
        polls.retain(|poll| poll.user != except_user);
        polls.sort_by(|left, right| left.user.cmp(&right.user));
        let now = Instant::now();
        for user_polls in polls.chunk_by(|left, right| left.user == right.user) {
            let user_id = user_polls[0].user.as_str();
            if let Some(slot) = self.users.lock(user_id).get_mut(user_id) {
                slot.signal(now, true, |parked| {
                    user_polls.iter().any(|poll| poll.id == parked.id)
                });
            }
        }
    }

    pub fn take_token(&self, user_id: &str, device: Uuid) -> bool {
        let now = Instant::now();
        let mut users = self.users.lock(user_id);
        let slot = users
            .entry(user_id.to_owned())
            .or_insert_with(|| UserSlot::new(now));
        slot.touched = now;
        slot.device(device, now).bucket.take(now)
    }

    pub fn focus_of(&self, user_id: &str, device: Uuid) -> Option<Uuid> {
        let users = self.users.lock(user_id);
        users
            .get(user_id)?
            .devices
            .iter()
            .find(|slot| slot.id == device)?
            .focus
    }

    pub fn set_focus(&self, user_id: &str, device: Uuid, focus: Option<Uuid>) {
        let now = Instant::now();
        let mut users = self.users.lock(user_id);
        let slot = users
            .entry(user_id.to_owned())
            .or_insert_with(|| UserSlot::new(now));
        slot.device(device, now).focus = focus;
    }

    pub fn seen_due(&self, user_id: &str) -> bool {
        let now = Instant::now();
        let mut users = self.users.lock(user_id);
        let slot = users
            .entry(user_id.to_owned())
            .or_insert_with(|| UserSlot::new(now));
        let due = slot
            .seen_at
            .is_none_or(|at| now.saturating_duration_since(at) >= SEEN_EVERY);
        if due {
            slot.seen_at = Some(now);
        }
        due
    }

    #[cfg(test)]
    pub fn tracked_users(&self) -> usize {
        (0..SHARDS)
            .map(|index| self.users.lock_at(index).len())
            .sum()
    }

    #[cfg(test)]
    pub fn prune_all(&self) {
        let now = Instant::now();
        for round in 0..SHARDS {
            self.prune(now, round);
        }
    }

    fn prune(&self, now: Instant, round: usize) {
        self.users.lock_at(round).retain(|_, slot| !slot.idle(now));
        self.typing.prune(now, round);
    }

    fn take(&self, user_id: &str, id: u64) -> Signal {
        let users = self.users.lock(user_id);
        let Some(poll) = users
            .get(user_id)
            .and_then(|slot| slot.polls.iter().find(|poll| poll.id == id))
        else {
            return Signal::ended();
        };
        let mut taken = Signal::default();
        poll.signal.send_if_modified(|signal| {
            taken = *signal;
            *signal = Signal {
                superseded: signal.superseded,
                ..Signal::default()
            };
            false
        });
        taken
    }

    fn unpark(&self, user_id: &str, id: u64, focus: Option<Uuid>) {
        if let Some(slot) = self.users.lock(user_id).get_mut(user_id) {
            slot.polls.retain(|poll| poll.id != id);
            slot.touched = Instant::now();
        }
        if let Some(conversation) = focus {
            let mut focused = self.focused.lock(&conversation);
            if let Some(polls) = focused.get_mut(&conversation) {
                polls.retain(|poll| poll.id != id);
                if polls.is_empty() {
                    focused.remove(&conversation);
                }
            }
        }
        self.parked.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for ChatHub {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Parked<'a> {
    hub: &'a ChatHub,
    user_id: String,
    id: u64,
    focus: Option<Uuid>,
    signal: watch::Receiver<Signal>,
}

impl Parked<'_> {
    pub async fn next(&mut self) -> Signal {
        if self.signal.changed().await.is_err() {
            return Signal::ended();
        }
        self.hub.take(&self.user_id, self.id)
    }

    pub fn superseded(&self) -> bool {
        self.signal.borrow().superseded
    }
}

impl Drop for Parked<'_> {
    fn drop(&mut self) {
        self.hub.unpark(&self.user_id, self.id, self.focus);
    }
}
