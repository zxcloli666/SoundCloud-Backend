use futures::StreamExt;
use uuid::Uuid;

use super::{Cause, RoomsService};
use crate::error::AppResult;
use crate::modules::rooms::model::{Perks, Room};
use crate::modules::rooms::store::Guard;

const RECHECK_PAGE: usize = 200;
const RECHECKS_IN_FLIGHT: usize = 8;

impl RoomsService {
    pub(super) async fn star_perks(&self, user_id: &str) -> Perks {
        let Some(chat) = self.chat() else {
            return Perks::free();
        };
        match chat.stars.is_star(user_id).await {
            Ok(star) => Perks::of(star),
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    "the STAR check of a room host failed, the room starts free"
                );
                Perks::free()
            }
        }
    }

    pub async fn live_since(&self, since: i64) -> AppResult<Vec<String>> {
        self.store.alive_since(since).await
    }

    pub async fn recheck_perks(&self, codes: &[String]) -> AppResult<u64> {
        let mut changed = 0;
        for page in codes.chunks(RECHECK_PAGE) {
            let rooms = self.store.load_many(page).await?;
            let outcomes: Vec<AppResult<bool>> = futures::stream::iter(rooms)
                .map(|room| self.recheck_room(room))
                .buffer_unordered(RECHECKS_IN_FLIGHT)
                .collect()
                .await;
            for outcome in outcomes {
                match outcome {
                    Ok(true) => changed += 1,
                    Ok(false) => {}
                    Err(error) => tracing::warn!(%error, "a room kept its perks this round"),
                }
            }
        }
        Ok(changed)
    }

    async fn recheck_room(&self, room: Room) -> AppResult<bool> {
        let Some(chat) = self.chat() else {
            return Ok(false);
        };
        let Ok(star) = chat.stars.is_star(&room.host_id).await else {
            return Ok(false);
        };
        let perks = Perks::of(star);
        if room.perks == perks {
            return Ok(false);
        }
        let host = room.host_id.clone();
        let (after, changed) = self
            .apply(&room.code, Cause::Quiet, |room| {
                if room.host_id != host || room.perks == perks {
                    return Ok(false);
                }
                room.perks = perks;
                Ok(true)
            })
            .await?;
        if changed {
            self.refresh_cards(&after).await;
        }
        Ok(changed)
    }

    pub async fn clear_image_backdrop(
        &self,
        code: &str,
        media: Uuid,
        cooldown_until: Option<i64>,
        guard: &Guard<'_>,
    ) -> AppResult<bool> {
        let Some(room) = self.store.load(code).await? else {
            return Ok(false);
        };
        if room.image_backdrop() != Some(media) {
            return Ok(false);
        }
        let (after, cleared) = self
            .apply_guarded(code, Cause::Quiet, Some(guard), |room| {
                Ok(room.clear_image_backdrop(media, cooldown_until))
            })
            .await?;
        self.refresh_cards(&after).await;
        Ok(cleared)
    }

    pub(super) async fn refresh_cards(&self, room: &Room) {
        self.previews.invalidate(&room.code);
        if room.public {
            *self.listing.lock().await = None;
        }
    }
}

#[cfg(test)]
#[path = "perks_tests.rs"]
pub(super) mod tests;
