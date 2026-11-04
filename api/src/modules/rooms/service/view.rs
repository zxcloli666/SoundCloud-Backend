use serde::Serialize;
use uuid::Uuid;

use crate::modules::rooms::model::{Member, Playback, Room};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub user_id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined_at: i64,
    pub ready_urn: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MuteView {
    pub user_id: String,
    pub until_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    pub code: String,
    pub host_id: String,
    pub version: u64,
    pub created_at: i64,
    pub members: Vec<MemberView>,
    pub playback: Playback,
    pub public: bool,
    pub title: Option<String>,
    pub chat_id: Option<Uuid>,
    pub chat_enabled: bool,
    pub moderators: Vec<String>,
    pub muted: Vec<MuteView>,
    pub slow_mode_s: u32,
    pub links: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banned: Option<Vec<String>>,
    pub online: Vec<String>,
    pub server_now: i64,
}

impl From<Member> for MemberView {
    fn from(member: Member) -> Self {
        Self {
            user_id: member.user_id,
            name: member.name,
            avatar_url: member.avatar_url,
            joined_at: member.joined_at,
            ready_urn: member.ready_urn,
        }
    }
}

impl RoomView {
    pub fn for_viewer(room: Room, viewer: &str, online: Vec<String>, now: i64) -> Self {
        let slow_mode_s = room.slow_mode();
        let banned = room.can_moderate(viewer).then(|| room.banned.clone());
        let muted = room
            .muted
            .iter()
            .filter(|mute| mute.until_ms > now)
            .map(|mute| MuteView {
                user_id: mute.user_id.clone(),
                until_ms: mute.until_ms,
            })
            .collect();
        Self {
            code: room.code,
            host_id: room.host_id,
            version: room.version,
            created_at: room.created_at,
            members: room.members.into_iter().map(MemberView::from).collect(),
            playback: room.playback,
            public: room.public,
            title: room.title,
            chat_id: room.chat_id,
            chat_enabled: room.chat_enabled,
            moderators: room.moderators,
            muted,
            slow_mode_s,
            links: room.links,
            banned,
            online,
            server_now: now,
        }
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
