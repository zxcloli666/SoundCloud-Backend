use axum::http::StatusCode;

use super::{Cause, ONLINE_WINDOW_MS, RoomsService, now_ms};
use crate::error::{AppError, AppResult};
use crate::modules::rooms::model::Room;

const LEAVE_ATTEMPTS: usize = 4;
const RACED: &str = "room_raced";

fn raced() -> AppError {
    AppError::coded(
        StatusCode::CONFLICT,
        RACED,
        "The room changed too quickly, try again",
    )
}

impl RoomsService {
    pub async fn leave(&self, code: &str, user_id: &str) -> AppResult<()> {
        let moved = match self.store.load(code).await? {
            Some(_) => None,
            None => self.store.moved_to(code).await?,
        };
        let code = moved.as_deref().unwrap_or(code);
        for _ in 0..LEAVE_ATTEMPTS {
            match self.leave_once(code, user_id).await {
                Err(AppError::Coded { code: RACED, .. }) => continue,
                outcome => return outcome,
            }
        }
        Err(raced())
    }

    async fn leave_once(&self, code: &str, user_id: &str) -> AppResult<()> {
        let Some(room) = self.store.load(code).await? else {
            return Ok(());
        };
        if !room.is_member(user_id) {
            return Ok(());
        }
        if room.members.len() == 1 {
            self.end(code).await?;
            return Ok(());
        }
        if room.is_host(user_id) {
            return self.hand_off(&room, user_id).await;
        }
        let (_, left) = self
            .apply(code, Cause::Member, |room| {
                if room.is_host(user_id) {
                    return Err(raced());
                }
                Ok(room.remove(user_id))
            })
            .await?;
        if left {
            self.store.forget(code, user_id).await?;
        }
        Ok(())
    }

    async fn hand_off(&self, room: &Room, host: &str) -> AppResult<()> {
        let candidates = self.host_candidates(room, host).await?;
        if room.next_host(host, &candidates).is_none() {
            self.end(&room.code).await?;
            return Ok(());
        }
        let (after, _) = self
            .apply(&room.code, Cause::Member, |room| {
                if !room.is_host(host) {
                    room.remove(host);
                    return Ok(());
                }
                room.hand_off(host, &candidates).map(drop).ok_or_else(raced)
            })
            .await?;
        self.store.forget(&after.code, host).await?;
        self.directory.release_host(host, &after.code).await?;
        self.directory.host_seen(&after, now_ms()).await?;
        self.previews.invalidate(&after.code);
        if after.public {
            *self.listing.lock().await = None;
        }
        Ok(())
    }

    async fn host_candidates(&self, room: &Room, host: &str) -> AppResult<Vec<String>> {
        let online = self
            .store
            .seen_since(&room.code, now_ms() - ONLINE_WINDOW_MS)
            .await?;
        let mut free = Vec::new();
        for user in online {
            if user == host || !room.is_member(&user) {
                continue;
            }
            let busy = match self.directory.hosted_by(&user).await? {
                Some(other) if other != room.code => self.store.version(&other).await?.is_some(),
                _ => false,
            };
            if !busy {
                free.push(user);
            }
        }
        Ok(free)
    }
}
