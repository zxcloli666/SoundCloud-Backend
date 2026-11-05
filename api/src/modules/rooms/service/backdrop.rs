use uuid::Uuid;

use super::{Cause, RoomsService, now_ms};
use crate::error::AppResult;
use crate::modules::rooms::model::Room;
use crate::modules::rooms::store::Guard;

impl RoomsService {
    pub async fn clear_image_backdrop(
        &self,
        code: &str,
        media: Uuid,
        guard: &Guard<'_>,
    ) -> AppResult<Option<Room>> {
        let Some(room) = self.store.load(code).await? else {
            return Ok(None);
        };
        if room.image_backdrop() != Some(media) {
            return Ok(None);
        }
        let (after, cleared) = self
            .apply_guarded(code, Cause::Quiet, Some(guard), |room| {
                Ok(room.clear_image_backdrop(media))
            })
            .await?;
        self.refresh_cards(&after).await;
        Ok(cleared.then_some(after))
    }

    pub async fn backdrop_cooldown(&self, host: &str) -> AppResult<Option<i64>> {
        self.store.cooling_until(host, now_ms()).await
    }

    pub async fn cool_down_backdrop(&self, host: &str, until: i64) -> AppResult<()> {
        self.store.cool_down(host, until, now_ms()).await
    }

    pub(super) async fn refresh_cards(&self, room: &Room) {
        self.previews.invalidate(&room.code);
        if room.public {
            *self.listing.lock().await = None;
        }
    }
}
