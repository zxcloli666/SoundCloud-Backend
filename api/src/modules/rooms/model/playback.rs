use serde_json::Value;

use super::{
    MAX_START_LEAD_MS, MAX_TRACK_BYTES, Playback, PlaybackStatus, PlaybackUpdate, Room, not_member,
};
use crate::common::sc_ids::{EntityKind, require_ref};
use crate::error::{AppError, AppResult};

const MIN_RATE: f64 = 0.5;
const MAX_RATE: f64 = 2.0;
const MAX_CROSSFADE_SEC: u32 = 12;

fn check_snapshot(track: &Value) -> AppResult<()> {
    let size = serde_json::to_vec(track)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if size > MAX_TRACK_BYTES || !track.is_object() {
        return Err(AppError::bad_request("track snapshot is too large"));
    }
    Ok(())
}

impl Room {
    pub fn set_playback(
        &mut self,
        user_id: &str,
        update: PlaybackUpdate,
        now: i64,
    ) -> AppResult<()> {
        if !self.is_host(user_id) {
            return Err(AppError::forbidden("Only the host controls playback"));
        }
        let track_urn = match update.track_urn.as_deref() {
            Some(urn) => Some(require_ref(EntityKind::Track, urn)?.urn()),
            None => None,
        };
        if track_urn.is_none() && update.status != PlaybackStatus::Idle {
            return Err(AppError::bad_request("trackUrn is required"));
        }
        for snapshot in [&update.track, &update.next_track].into_iter().flatten() {
            check_snapshot(snapshot)?;
        }
        let same_track = track_urn == self.playback.track_urn;
        let track = match update.track {
            Some(track) => Some(track),
            None if same_track => self.playback.track.take(),
            None => None,
        };
        let next_track = match update.next_track {
            Some(next) => Some(next),
            None if same_track => self.playback.next_track.take(),
            None => None,
        };
        self.playback = Playback {
            status: update.status,
            track,
            track_urn,
            next_track,
            position_ms: update.position_ms.max(0),
            at: now + update.lead_ms.clamp(0, MAX_START_LEAD_MS),
            rate: update.rate.unwrap_or(1.0).clamp(MIN_RATE, MAX_RATE),
            crossfade_sec: update
                .crossfade_sec
                .unwrap_or(self.playback.crossfade_sec)
                .min(MAX_CROSSFADE_SEC),
        };
        Ok(())
    }

    pub fn mark_ready(&mut self, user_id: &str, track_urn: &str) -> AppResult<bool> {
        let urn = require_ref(EntityKind::Track, track_urn)?.urn();
        let member = self
            .members
            .iter_mut()
            .find(|m| m.user_id == user_id)
            .ok_or_else(not_member)?;
        if member.ready_urn.as_deref() == Some(urn.as_str()) {
            return Ok(false);
        }
        member.ready_urn = Some(urn);
        Ok(true)
    }
}
