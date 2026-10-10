use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use deadpool_redis::Pool;
use serde::Serialize;
use tokio::time::Instant;

use crate::error::{AppError, AppResult};
use crate::modules::rooms::directory::RoomDirectory;
use crate::modules::rooms::hub::RoomHub;
use crate::modules::rooms::listing::PublicRoom;
use crate::modules::rooms::model::{Member, PlaybackUpdate, Profile, Room, new_code, not_member};
use crate::modules::rooms::store::RoomStore;

pub const ONLINE_WINDOW_MS: i64 = 45_000;
const RECHECK_EVERY: Duration = Duration::from_secs(2);
const CODE_ATTEMPTS: usize = 6;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    #[serde(flatten)]
    pub room: Room,
    pub online: Vec<String>,
    pub server_now: i64,
}

pub struct RoomsService {
    pub(super) store: RoomStore,
    pub(super) directory: RoomDirectory,
    pub(super) hub: Arc<RoomHub>,
    pub(super) listing: tokio::sync::Mutex<Option<(Instant, Arc<Vec<PublicRoom>>)>>,
}

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl RoomsService {
    pub fn new(redis: Pool, hub: Arc<RoomHub>) -> Arc<Self> {
        Arc::new(Self {
            store: RoomStore::new(redis.clone()),
            directory: RoomDirectory::new(redis),
            hub,
            listing: tokio::sync::Mutex::new(None),
        })
    }

    pub async fn create(
        &self,
        user_id: &str,
        profile: &Profile,
        public: bool,
    ) -> AppResult<RoomView> {
        if let Some(previous) = self.directory.hosted_by(user_id).await? {
            self.leave(&previous, user_id).await?;
        }
        let now = now_ms();
        for _ in 0..CODE_ATTEMPTS {
            let host = Member::new(user_id, profile, now);
            let room = Room::new(new_code(), host, public, now);
            if self.store.insert(&room).await? {
                self.store.touch(&room.code, user_id, now).await?;
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

    pub async fn view(&self, code: &str, user_id: &str) -> AppResult<RoomView> {
        let room = self
            .store
            .load(code)
            .await?
            .ok_or_else(|| AppError::not_found("Room not found"))?;
        if !room.is_member(user_id) {
            return Err(not_member());
        }
        let now = now_ms();
        if room.is_host(user_id) {
            self.directory.host_seen(&room, now).await?;
        }
        let online = self.store.seen_since(code, now - ONLINE_WINDOW_MS).await?;
        Ok(RoomView {
            room,
            online,
            server_now: now,
        })
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
        self.store.touch(code, user_id, now_ms()).await?;
        if let Some(since) = since {
            let deadline = Instant::now() + hold;
            loop {
                let version = self
                    .store
                    .version(code)
                    .await?
                    .ok_or_else(|| AppError::not_found("Room not found"))?;
                let now = Instant::now();
                if version != since || now >= deadline {
                    break;
                }
                let pause = RECHECK_EVERY.min(deadline - now);
                tokio::select! {
                    () = changes.changed(code) => {}
                    () = tokio::time::sleep(pause) => {}
                }
            }
        }
        self.view(code, user_id).await
    }

    pub async fn join(&self, code: &str, user_id: &str, profile: &Profile) -> AppResult<RoomView> {
        self.admit(code, user_id, profile, false).await
    }

    pub(super) async fn admit(
        &self,
        code: &str,
        user_id: &str,
        profile: &Profile,
        public_only: bool,
    ) -> AppResult<RoomView> {
        let now = now_ms();
        let member = Member::new(user_id, profile, now);
        let recent = now - ONLINE_WINDOW_MS;
        let online = self.store.seen_since(code, recent).await?;
        self.store
            .update(code, |room| {
                let known = room.is_member(user_id);
                if public_only && !room.public && !known {
                    return Err(AppError::not_found("Room not found"));
                }
                if room.is_full() && !known {
                    room.drop_absent(&online, recent);
                }
                room.join(member.clone())
            })
            .await?;
        self.store.touch(code, user_id, now_ms()).await?;
        self.hub.announce(code).await;
        self.view(code, user_id).await
    }

    pub async fn leave(&self, code: &str, user_id: &str) -> AppResult<()> {
        let moved = match self.store.load(code).await? {
            Some(_) => None,
            None => self.store.moved_to(code).await?,
        };
        let code = moved.as_deref().unwrap_or(code);
        let Some(room) = self.store.load(code).await? else {
            return Ok(());
        };
        if room.is_host(user_id) {
            self.store.delete(code).await?;
            self.directory.close(&room).await?;
            *self.listing.lock().await = None;
        } else {
            let (_, left) = self
                .store
                .update(code, |room| Ok(room.remove(user_id)))
                .await?;
            if !left {
                return Ok(());
            }
            self.store.forget(code, user_id).await?;
        }
        self.hub.announce(code).await;
        Ok(())
    }

    pub async fn set_playback(
        &self,
        code: &str,
        user_id: &str,
        update: PlaybackUpdate,
    ) -> AppResult<RoomView> {
        self.store
            .update(code, |room| {
                room.set_playback(user_id, update.clone(), now_ms())
            })
            .await?;
        self.hub.announce(code).await;
        self.view(code, user_id).await
    }

    pub async fn mark_ready(
        &self,
        code: &str,
        user_id: &str,
        track_urn: &str,
    ) -> AppResult<RoomView> {
        let mut room = self
            .store
            .load(code)
            .await?
            .ok_or_else(|| AppError::not_found("Room not found"))?;
        if room.mark_ready(user_id, track_urn)? {
            self.store
                .update(code, |room| room.mark_ready(user_id, track_urn))
                .await?;
            self.hub.announce(code).await;
        }
        self.view(code, user_id).await
    }
}
