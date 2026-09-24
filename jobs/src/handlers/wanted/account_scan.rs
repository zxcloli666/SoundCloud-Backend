use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;
use tracing::{debug, info};
use uuid::Uuid;

use super::{LINK_THRESHOLD, WantedHandler, WantedTrack, triage};

const PAGE_SIZE: i64 = 100;
const MAX_PAGES: usize = 20;
const PAGE_GAP: Duration = Duration::from_millis(150);
const ACCOUNT_LINK_THRESHOLD: f32 = LINK_THRESHOLD;

impl WantedHandler {
    pub(super) async fn scan_artist_uploads(
        &self,
        artist_id: Uuid,
        wanted: &[&WantedTrack],
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        let accounts = sqlx::query_file_scalar!("queries/wanted/identity_accounts.sql", artist_id)
            .fetch_all(&self.pool)
            .await?;
        if accounts.is_empty() {
            return Ok(Vec::new());
        }

        let mut linked: Vec<Uuid> = Vec::new();
        let mut settled: HashSet<Uuid> = HashSet::new();

        for sc_user_id in accounts {
            if settled.len() == wanted.len() {
                break;
            }
            let uploads = self.account_uploads(&sc_user_id).await;
            if uploads.is_empty() {
                continue;
            }
            info!(
                %artist_id,
                sc_user_id,
                uploads = uploads.len(),
                pending = wanted.len() - settled.len(),
                "matching identity account uploads against wanted tracks"
            );

            for track in wanted {
                if settled.contains(&track.id) {
                    continue;
                }
                let Some((index, score)) = self.pick_upload(&uploads, track).await else {
                    continue;
                };
                let Some(upload) = uploads.get(index) else {
                    continue;
                };
                match self
                    .ingest_and_link(track, upload, score, "identity_account")
                    .await
                {
                    Ok(true) => {
                        settled.insert(track.id);
                        linked.push(track.id);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        debug!(wanted = %track.id, %error, "linking an identity upload failed")
                    }
                }
            }
        }
        Ok(linked)
    }

    async fn pick_upload(&self, uploads: &[Value], track: &WantedTrack) -> Option<(usize, f32)> {
        let triaged = triage(uploads, track, ACCOUNT_LINK_THRESHOLD);
        if let Some(best) = triaged.best {
            return Some(best);
        }
        self.ask_ai(track, uploads, &triaged.borderline)
            .await
            .ok()
            .flatten()
    }

    async fn account_uploads(&self, sc_user_id: &str) -> Vec<Value> {
        let path = format!("/users/{sc_user_id}/tracks?access=playable,preview,blocked");
        let mut uploads: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;

        for page_index in 0..MAX_PAGES {
            if page_index > 0 {
                tokio::time::sleep(PAGE_GAP).await;
            }
            let page = match self
                .reader
                .list_page(&path, cursor.as_deref(), PAGE_SIZE, 0)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    debug!(sc_user_id, %error, "identity account page unavailable");
                    break;
                }
            };
            if page.items.is_empty() {
                break;
            }
            uploads.extend(page.items);
            match page.next_href {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => break,
            }
        }
        uploads
    }
}
