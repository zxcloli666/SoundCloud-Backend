use axum::http::StatusCode;

use super::{Cause, ONLINE_WINDOW_MS, RoomsService, now_ms};
use crate::error::{AppError, AppResult};
use crate::modules::rooms::model::{Perks, Room};

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
        let Some(expected) = room.next_host(host, &candidates) else {
            self.end(&room.code).await?;
            return Ok(());
        };
        let perks = self.star_perks(&expected).await;
        let (after, surprise) = self
            .apply(&room.code, Cause::Member, |room| {
                if !room.is_host(host) {
                    room.remove(host);
                    return Ok(None);
                }
                let next = room.hand_off(host, &candidates).ok_or_else(raced)?;
                if next == expected {
                    room.perks = perks;
                    return Ok(None);
                }
                room.perks = Perks::free();
                Ok(Some(next))
            })
            .await?;
        if let Some(next) = surprise {
            self.settle_perks(&after.code, &next).await;
        }
        self.store.forget(&after.code, host).await?;
        self.directory.release_host(host, &after.code).await?;
        self.directory.host_seen(&after, now_ms()).await?;
        self.refresh_cards(&after).await;
        Ok(())
    }

    async fn settle_perks(&self, code: &str, host: &str) {
        let perks = self.star_perks(host).await;
        let settled = self
            .apply(code, Cause::Quiet, |room| {
                if room.host_id == host {
                    room.perks = perks;
                }
                Ok(())
            })
            .await;
        if let Err(error) = settled {
            tracing::warn!(code, %error, "the new host's perks wait for the STAR recheck");
        }
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

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod tests;
