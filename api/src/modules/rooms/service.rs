mod admit;
mod alive;
mod apply;
mod chat;
mod end;
mod handoff;
mod moderation;
mod preview;
mod view;

use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use axum::http::StatusCode;
use deadpool_redis::Pool;
use mini_moka::sync::Cache;
use tokio::time::Instant;

use crate::error::{AppError, AppResult};
use crate::modules::chat::ChatState;
use crate::modules::rooms::directory::RoomDirectory;
use crate::modules::rooms::hub::RoomHub;
use crate::modules::rooms::listing::PublicRoom;
use crate::modules::rooms::model::{
    Member, PlaybackUpdate, Profile, Room, clean_title, new_code, not_member,
};
use crate::modules::rooms::store::RoomStore;

pub use apply::Cause;
pub use chat::OpenedChat;
pub use preview::RoomPreview;
pub use view::RoomView;

pub const ONLINE_WINDOW_MS: i64 = 45_000;
const RECHECK_EVERY: Duration = Duration::from_secs(2);
const CODE_ATTEMPTS: usize = 6;
const ALIVE_EVERY: Duration = Duration::from_secs(60);
const PREVIEW_FRESH_FOR: Duration = Duration::from_secs(10);
const CACHED_ROOMS: u64 = 20_000;

pub struct RoomsService {
    pub(super) store: RoomStore,
    pub(super) directory: RoomDirectory,
    pub(super) hub: Arc<RoomHub>,
    pub(super) listing: tokio::sync::Mutex<Option<(Instant, Arc<Vec<PublicRoom>>)>>,
    chat: OnceLock<Weak<ChatState>>,
    alive: Cache<String, ()>,
    previews: Cache<String, Option<Arc<RoomPreview>>>,
}

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn expiring<V: Clone + Send + Sync + 'static>(ttl: Duration) -> Cache<String, V> {
    Cache::builder()
        .time_to_live(ttl)
        .max_capacity(CACHED_ROOMS)
        .build()
}

impl RoomsService {
    pub fn new(redis: Pool, hub: Arc<RoomHub>) -> Arc<Self> {
        Arc::new(Self {
            store: RoomStore::new(redis.clone()),
            directory: RoomDirectory::new(redis),
            hub,
            listing: tokio::sync::Mutex::new(None),
            chat: OnceLock::new(),
            alive: expiring(ALIVE_EVERY),
            previews: expiring(PREVIEW_FRESH_FOR),
        })
    }

    pub fn install_chat(&self, chat: &Arc<ChatState>) {
        let _ = self.chat.set(Arc::downgrade(chat));
    }

    fn chat(&self) -> Option<Arc<ChatState>> {
        self.chat.get()?.upgrade()
    }

    pub async fn create(
        &self,
        user_id: &str,
        profile: &Profile,
        public: bool,
        title: Option<&str>,
    ) -> AppResult<RoomView> {
        if let Some(previous) = self.directory.hosted_by(user_id).await? {
            self.leave(&previous, user_id).await?;
        }
        let now = now_ms();
        for _ in 0..CODE_ATTEMPTS {
            let host = Member::new(user_id, profile, now);
            let mut room = Room::new(new_code(), host, public, now);
            room.title = title.and_then(clean_title);
            if self.store.insert(&room).await? {
                self.store.touch(&room.code, user_id, now).await?;
                self.keep_alive(&room, now).await?;
                if public {
                    *self.listing.lock().await = None;
                }
                return self.view(&room.code, user_id).await;
            }
        }
        Err(AppError::service_unavailable(
            "Could not allocate a room code",
        ))
    }

    pub async fn find(&self, code: &str) -> AppResult<Option<Room>> {
        self.store.load(code).await
    }

    pub async fn room(&self, code: &str) -> AppResult<Room> {
        self.store
            .load(code)
            .await?
            .ok_or_else(|| AppError::not_found("Room not found"))
    }

    pub async fn view(&self, code: &str, user_id: &str) -> AppResult<RoomView> {
        let room = self.room(code).await?;
        self.show(room, user_id).await
    }

    async fn show(&self, room: Room, user_id: &str) -> AppResult<RoomView> {
        if !room.is_member(user_id) {
            return Err(not_member());
        }
        let now = now_ms();
        if room.is_host(user_id) {
            self.directory.host_seen(&room, now).await?;
        }
        let online = self
            .store
            .seen_since(&room.code, now - ONLINE_WINDOW_MS)
            .await?;
        Ok(RoomView::for_viewer(room, user_id, online, now))
    }

    pub async fn wait(
        &self,
        code: &str,
        user_id: &str,
        since: Option<u64>,
        hold: Duration,
        follow: bool,
    ) -> AppResult<RoomView> {
        let error = match self.wait_on(code, user_id, since, hold).await {
            Err(error) if follow && error.status() == StatusCode::NOT_FOUND => error,
            outcome => return outcome,
        };
        let Some(moved) = self.store.moved_to(code).await? else {
            return Err(error);
        };
        let view = self.view(&moved, user_id).await?;
        self.store.touch(&moved, user_id, now_ms()).await?;
        Ok(view)
    }

    async fn wait_on(
        &self,
        code: &str,
        user_id: &str,
        since: Option<u64>,
        hold: Duration,
    ) -> AppResult<RoomView> {
        let mut changes = self.hub.listen().await;
        let room = self.room(code).await?;
        if !room.is_member(user_id) {
            return Err(not_member());
        }
        let now = now_ms();
        self.store.touch(code, user_id, now).await?;
        self.keep_alive(&room, now).await?;
        let mut version = room.version;
        if let Some(since) = since {
            let deadline = Instant::now() + hold;
            loop {
                let now = Instant::now();
                if version != since || now >= deadline {
                    break;
                }
                let left = deadline - now;
                let pause = if self.hub.is_shared() {
                    RECHECK_EVERY.min(left)
                } else {
                    left
                };
                tokio::select! {
                    () = changes.changed(code) => {}
                    () = tokio::time::sleep(pause) => {}
                }
                version = self
                    .store
                    .version(code)
                    .await?
                    .ok_or_else(|| AppError::not_found("Room not found"))?;
            }
        }
        if version == room.version {
            return self.show(room, user_id).await;
        }
        self.view(code, user_id).await
    }

    pub async fn set_playback(
        &self,
        code: &str,
        user_id: &str,
        update: PlaybackUpdate,
    ) -> AppResult<RoomView> {
        self.apply(code, Cause::Member, |room| {
            room.set_playback(user_id, update.clone(), now_ms())
        })
        .await?;
        self.view(code, user_id).await
    }

    pub async fn mark_ready(
        &self,
        code: &str,
        user_id: &str,
        track_urn: &str,
    ) -> AppResult<RoomView> {
        let mut room = self.room(code).await?;
        if room.mark_ready(user_id, track_urn)? {
            self.apply(code, Cause::Member, |room| {
                room.mark_ready(user_id, track_urn)
            })
            .await?;
        }
        self.view(code, user_id).await
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
