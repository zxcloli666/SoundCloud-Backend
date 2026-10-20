use sqlx::PgPool;

use super::{Cause, ONLINE_WINDOW_MS, RoomView, RoomsService, now_ms};
use crate::error::{AppError, AppResult};
use crate::modules::rooms::model::{Member, Profile, Room, banned};
use crate::modules::rooms::public::host_blocks;

fn hidden() -> AppError {
    AppError::not_found("Room not found")
}

fn refusal(room: &Room, user_id: &str, public_only: bool) -> Option<AppError> {
    if public_only && !room.public {
        return Some(hidden());
    }
    if room.is_banned(user_id) {
        return Some(if public_only { hidden() } else { banned() });
    }
    None
}

impl RoomsService {
    pub async fn join(
        &self,
        code: &str,
        user_id: &str,
        profile: &Profile,
        pg: &PgPool,
    ) -> AppResult<RoomView> {
        self.admit(code, user_id, profile, false, pg).await
    }

    pub(in crate::modules::rooms) async fn admit(
        &self,
        code: &str,
        user_id: &str,
        profile: &Profile,
        public_only: bool,
        pg: &PgPool,
    ) -> AppResult<RoomView> {
        let room = self.room(code).await?;
        let known = room.is_member(user_id);
        if !known {
            if let Some(error) = refusal(&room, user_id, public_only) {
                return Err(error);
            }
            if host_blocks(pg, &room.host_id, user_id).await? {
                return Err(hidden());
            }
        }
        let from_seq = if known {
            0
        } else {
            self.chat_last_seq(&room).await
        };
        let now = now_ms();
        let member = Member::new(user_id, profile, now).reading_from(from_seq);
        let recent = now - ONLINE_WINDOW_MS;
        let online = self.store.seen_since(code, recent).await?;
        let checked_host = room.host_id;
        self.apply(code, Cause::Member, |room| {
            if !room.is_member(user_id) {
                if let Some(error) = refusal(room, user_id, public_only) {
                    return Err(error);
                }
                if room.host_id != checked_host {
                    return Err(AppError::conflict(
                        "The room changed too quickly, try again",
                    ));
                }
                if room.is_full() {
                    room.drop_absent(&online, recent);
                }
            }
            room.join(member.clone())
        })
        .await?;
        self.store.touch(code, user_id, now_ms()).await?;
        self.view(code, user_id).await
    }
}
