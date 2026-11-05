use futures::StreamExt;

use super::{Cause, RoomsService};
use crate::error::AppResult;
use crate::modules::rooms::model::{Perks, Room};

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

    pub async fn upgrade_to_star(&self, code: &str, host: &str) -> AppResult<Room> {
        let (after, upgraded) = self
            .apply(code, Cause::Quiet, |room| {
                if !room.is_host(host) || room.perks.star_host {
                    return Ok(false);
                }
                room.perks = Perks::star();
                Ok(true)
            })
            .await?;
        if upgraded {
            self.refresh_cards(&after).await;
        }
        Ok(after)
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
}

#[cfg(test)]
#[path = "perks_tests.rs"]
pub(super) mod tests;
