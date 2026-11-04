use super::{Cause, RoomView, RoomsService, now_ms};
use crate::error::AppResult;
use crate::modules::rooms::model::SettingsPatch;

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

    pub async fn configure(
        &self,
        code: &str,
        actor: &str,
        patch: &SettingsPatch,
    ) -> AppResult<RoomView> {
        let cause = Cause::Moderation {
            by: actor,
            ban: false,
        };
        let (after, _) = self
            .apply(code, cause, |room| room.apply_settings(actor, patch))
            .await?;
        self.previews.invalidate(&after.code);
        if after.public {
            *self.listing.lock().await = None;
        }
        self.view(code, actor).await
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
