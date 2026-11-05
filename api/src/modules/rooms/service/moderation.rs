use uuid::Uuid;

use super::{Cause, RoomsService, now_ms};
use crate::error::AppResult;
use crate::modules::rooms::model::{Room, SettingsPatch};
use crate::modules::rooms::store::Guard;

impl RoomsService {
    pub async fn kick(&self, code: &str, actor: &str, target: &str, ban: bool) -> AppResult<()> {
        let cause = Cause::Moderation { by: actor, ban };
        let (after, _) = self
            .apply(code, cause, |room| {
                if ban {
                    room.ban(actor, target)
                } else {
                    room.kick(actor, target)
                }
            })
            .await?;
        self.store.forget(&after.code, target).await
    }

    pub async fn mute(&self, code: &str, actor: &str, target: &str, seconds: u32) -> AppResult<()> {
        let cause = Cause::Moderation {
            by: actor,
            ban: false,
        };
        self.apply(code, cause, |room| {
            room.mute(actor, target, seconds, now_ms())
        })
        .await?;
        Ok(())
    }

    pub async fn set_moderator(
        &self,
        code: &str,
        actor: &str,
        target: &str,
        on: bool,
    ) -> AppResult<()> {
        let cause = Cause::Moderation {
            by: actor,
            ban: false,
        };
        self.apply(code, cause, |room| room.set_moderator(actor, target, on))
            .await?;
        Ok(())
    }

    pub async fn reconfigure(
        &self,
        code: &str,
        actor: &str,
        patch: &SettingsPatch,
    ) -> AppResult<Room> {
        let (after, _) = self
            .reconfigure_guarded(code, actor, patch, None, None)
            .await?;
        Ok(after)
    }

    pub async fn reconfigure_guarded(
        &self,
        code: &str,
        actor: &str,
        patch: &SettingsPatch,
        cooldown: Option<i64>,
        guard: Option<&Guard<'_>>,
    ) -> AppResult<(Room, Option<Uuid>)> {
        let cause = Cause::Moderation {
            by: actor,
            ban: false,
        };
        let (after, replaced) = self
            .apply_guarded(code, cause, guard, |room| {
                let before = room.image_backdrop();
                room.apply_settings(actor, patch, cooldown)?;
                Ok(before.filter(|_| room.image_backdrop() != before))
            })
            .await?;
        self.refresh_cards(&after).await;
        Ok((after, replaced))
    }

    pub async fn unlist(&self, code: &str) -> AppResult<bool> {
        if self.store.load(code).await?.is_none() {
            return Ok(false);
        }
        self.apply(code, Cause::Quiet, |room| {
            room.unlisted = true;
            Ok(())
        })
        .await?;
        self.directory.unlist(code).await?;
        *self.listing.lock().await = None;
        Ok(true)
    }
}
