use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::time::Instant;

use crate::common::sc_ids::user_id_variants;
use crate::error::{AppError, AppResult};
use crate::modules::rooms::listing::{PublicRoom, rank};
use crate::modules::rooms::model::Profile;
use crate::modules::rooms::service::{ONLINE_WINDOW_MS, RoomView, RoomsService, now_ms};

pub const LIST_LIMIT: usize = 50;
const SCAN_LIMIT: isize = 300;
const LIST_FRESH_FOR: Duration = Duration::from_secs(5);
const MAX_HOST_ID_LEN: usize = 20;

fn host_id_of(raw: &str) -> AppResult<&str> {
    let valid =
        !raw.is_empty() && raw.len() <= MAX_HOST_ID_LEN && raw.bytes().all(|b| b.is_ascii_digit());
    valid
        .then_some(raw)
        .ok_or_else(|| AppError::not_found("Room not found"))
}

pub async fn blocked_hosts(pg: &PgPool, sc_user_id: &str) -> AppResult<HashSet<String>> {
    let variants = user_id_variants(sc_user_id);
    let rows = sqlx::query_file_scalar!("queries/rooms/blocked_hosts.sql", &variants)
        .fetch_all(pg)
        .await?;
    Ok(rows.into_iter().collect())
}

impl RoomsService {
    pub async fn set_public(&self, code: &str, user_id: &str, public: bool) -> AppResult<RoomView> {
        let (room, ()) = self
            .store
            .update(code, |room| room.set_public(user_id, public))
            .await?;
        if public {
            self.directory.host_seen(&room, now_ms()).await?;
        } else {
            self.directory.unlist(code).await?;
        }
        *self.listing.lock().await = None;
        self.hub.announce(code).await;
        self.view(code, user_id).await
    }

    pub async fn public_rooms(&self) -> AppResult<Arc<Vec<PublicRoom>>> {
        let mut listing = self.listing.lock().await;
        if let Some((at, rooms)) = listing.as_ref()
            && at.elapsed() < LIST_FRESH_FOR
        {
            return Ok(rooms.clone());
        }
        let since = now_ms() - ONLINE_WINDOW_MS;
        let codes = self.directory.listed_since(since, SCAN_LIMIT).await?;
        let mut rooms: Vec<PublicRoom> = self
            .store
            .load_with_presence(&codes, since)
            .await?
            .iter()
            .filter(|(room, _)| room.public)
            .filter_map(|(room, online)| PublicRoom::of(room, online))
            .collect();
        rank(&mut rooms);
        rooms.truncate(LIST_LIMIT);
        let rooms = Arc::new(rooms);
        *listing = Some((Instant::now(), rooms.clone()));
        Ok(rooms)
    }

    pub async fn join_public(
        &self,
        host_id: &str,
        user_id: &str,
        profile: &Profile,
    ) -> AppResult<RoomView> {
        let code = self
            .directory
            .hosted_by(host_id_of(host_id)?)
            .await?
            .ok_or_else(|| AppError::not_found("Room not found"))?;
        let view = self.admit(&code, user_id, profile, true).await?;
        if view.room.host_id != host_id {
            return Err(AppError::not_found("Room not found"));
        }
        Ok(view)
    }
}
