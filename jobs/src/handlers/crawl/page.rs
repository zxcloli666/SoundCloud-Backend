use std::collections::{HashMap, HashSet};

use catalog_normalize::{NORMALIZER_VERSION, TitleForms};
use sqlx::PgPool;
use uuid::Uuid;

use super::entities;
use super::error::CrawlResult;

const FLUSH_AT: usize = 50;

pub struct WantedDraft {
    pub title: String,
    pub normalized: String,
    pub primary_artist_id: Option<Uuid>,
    pub external_id: String,
    pub forms: TitleForms,
    pub featured: Vec<(Uuid, i16)>,
    pub album: Option<(Uuid, i16)>,
}

#[derive(Default)]
pub struct Page {
    artists: HashMap<(Option<String>, String), Option<Uuid>>,
    drafts: Vec<WantedDraft>,
    external_ids: HashSet<String>,
}

impl Page {
    pub async fn artist_id(
        &mut self,
        pool: &PgPool,
        genius_artist_id: Option<&str>,
        name: &str,
    ) -> CrawlResult<Option<Uuid>> {
        let key = (genius_artist_id.map(str::to_owned), name.to_owned());
        if let Some(found) = self.artists.get(&key) {
            return Ok(*found);
        }
        let found = entities::artist_id(pool, None, genius_artist_id, name).await?;
        self.artists.insert(key, found);
        Ok(found)
    }

    pub async fn add(&mut self, pool: &PgPool, draft: WantedDraft) -> CrawlResult {
        if self.external_ids.contains(&draft.external_id) {
            self.flush(pool).await?;
        }
        self.external_ids.insert(draft.external_id.clone());
        self.drafts.push(draft);
        if self.drafts.len() >= FLUSH_AT {
            self.flush(pool).await?;
        }
        Ok(())
    }

    pub async fn flush(&mut self, pool: &PgPool) -> CrawlResult {
        let drafts = std::mem::take(&mut self.drafts);
        self.external_ids.clear();
        if drafts.is_empty() {
            return Ok(());
        }
        let ids = upsert(pool, &drafts).await?;

        let mut wanted_ids = Vec::new();
        let mut alias_wanted = Vec::new();
        let mut alias_keys = Vec::new();
        let mut credit_wanted = Vec::new();
        let mut credit_artist = Vec::new();
        let mut credit_role = Vec::new();
        let mut credit_position = Vec::new();
        let mut place_wanted = Vec::new();
        let mut place_album = Vec::new();
        let mut place_position = Vec::new();
        for draft in &drafts {
            let Some(&wanted_id) = ids.get(&draft.external_id) else {
                continue;
            };
            wanted_ids.push(wanted_id);
            for alias in &draft.forms.aliases {
                alias_wanted.push(wanted_id);
                alias_keys.push(alias.clone());
            }
            if let Some(artist_id) = draft.primary_artist_id {
                credit_wanted.push(wanted_id);
                credit_artist.push(artist_id);
                credit_role.push("primary".to_owned());
                credit_position.push(0i16);
            }
            for &(artist_id, position) in &draft.featured {
                credit_wanted.push(wanted_id);
                credit_artist.push(artist_id);
                credit_role.push("featured".to_owned());
                credit_position.push(position);
            }
            if let Some((album_id, position)) = draft.album {
                place_wanted.push(wanted_id);
                place_album.push(album_id);
                place_position.push(position);
            }
        }
        sqlx::query_file!(
            "queries/crawl/replace_wanted_aliases_batch.sql",
            &wanted_ids,
            &alias_wanted,
            &alias_keys
        )
        .execute(pool)
        .await?;
        sqlx::query_file!(
            "queries/crawl/insert_wanted_track_artists_batch.sql",
            &credit_wanted,
            &credit_artist,
            &credit_role,
            &credit_position
        )
        .execute(pool)
        .await?;
        sqlx::query_file!(
            "queries/crawl/insert_wanted_track_albums_batch.sql",
            &place_wanted,
            &place_album,
            &place_position
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

async fn upsert(pool: &PgPool, drafts: &[WantedDraft]) -> CrawlResult<HashMap<String, Uuid>> {
    let titles: Vec<String> = drafts.iter().map(|draft| draft.title.clone()).collect();
    let normalized: Vec<String> = drafts
        .iter()
        .map(|draft| draft.normalized.clone())
        .collect();
    let artists: Vec<Option<Uuid>> = drafts.iter().map(|draft| draft.primary_artist_id).collect();
    let external_ids: Vec<String> = drafts
        .iter()
        .map(|draft| draft.external_id.clone())
        .collect();
    let work_keys: Vec<String> = drafts
        .iter()
        .map(|draft| draft.forms.work_key.clone())
        .collect();
    let recording_keys: Vec<String> = drafts
        .iter()
        .map(|draft| draft.forms.recording_key.clone())
        .collect();
    let rows = sqlx::query_file!(
        "queries/crawl/upsert_wanted_from_genius_batch.sql",
        &titles,
        &normalized,
        &artists as &[Option<Uuid>],
        &external_ids,
        &work_keys,
        &recording_keys,
        NORMALIZER_VERSION
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.external_id, row.id))
        .collect())
}
