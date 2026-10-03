use std::collections::HashMap;

use serde_json::Value;
use sqlx::PgPool;

use super::query::LiveKind;
use crate::common::sc_ids::extract_sc_id;
use crate::error::AppResult;
use crate::modules::playlists::PlaylistRow;
use crate::modules::search::repository::project_playlists_with_owners;
use crate::modules::users::{UserRow, project_to_sc_shape as project_user};

pub type Serving = HashMap<String, Option<Value>>;

pub async fn lookup(pg: &PgPool, kind: LiveKind, urns: &[String]) -> AppResult<Serving> {
    if urns.is_empty() {
        return Ok(Serving::new());
    }
    let urn_by_id: HashMap<String, String> = urns
        .iter()
        .map(|urn| (extract_sc_id(urn).to_owned(), urn.clone()))
        .collect();
    let ids: Vec<String> = urn_by_id.keys().cloned().collect();
    let found = match kind {
        LiveKind::Tracks => tracks(pg, &ids).await?,
        LiveKind::Users => users(pg, &ids).await?,
        LiveKind::Playlists => playlists(pg, &ids).await?,
    };
    Ok(found
        .into_iter()
        .filter_map(|(id, local)| Some((urn_by_id.get(&id)?.clone(), local)))
        .collect())
}

async fn tracks(pg: &PgPool, ids: &[String]) -> AppResult<Serving> {
    let rows = sqlx::query_file!("queries/live_search/tracks_by_sc_ids.sql", ids)
        .fetch_all(pg)
        .await?;
    let mut winners: Vec<String> = rows.iter().filter_map(|row| row.serving.clone()).collect();
    winners.sort();
    winners.dedup();
    let projected = crate::modules::tracks::project_many_public(pg, &winners).await?;
    let by_winner: HashMap<String, Value> = winners
        .into_iter()
        .zip(projected)
        .filter_map(|(id, track)| Some((id, track?)))
        .collect();
    Ok(rows
        .into_iter()
        .map(|row| {
            let local = row
                .serving
                .and_then(|winner| by_winner.get(&winner).cloned());
            (row.requested, local)
        })
        .collect())
}

async fn users(pg: &PgPool, ids: &[String]) -> AppResult<Serving> {
    let rows = sqlx::query_file_as!(
        UserRow,
        "queries/search/repository/users_by_sc_ids.sql",
        ids
    )
    .fetch_all(pg)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.sc_user_id.clone(), Some(project_user(row))))
        .collect())
}

async fn playlists(pg: &PgPool, ids: &[String]) -> AppResult<Serving> {
    let rows = sqlx::query_file_as!(
        PlaylistRow,
        "queries/live_search/playlists_by_sc_ids.sql",
        ids
    )
    .fetch_all(pg)
    .await?;
    let (serving, hidden): (Vec<PlaylistRow>, Vec<PlaylistRow>) = rows
        .into_iter()
        .partition(|row| row.sharing == "public" && row.deleted_at.is_none());
    let serving_ids: Vec<String> = serving
        .iter()
        .map(|row| row.sc_playlist_id.clone())
        .collect();
    let projected = project_playlists_with_owners(pg, serving).await?;
    Ok(serving_ids
        .into_iter()
        .zip(projected)
        .map(|(id, playlist)| (id, Some(playlist)))
        .chain(hidden.into_iter().map(|row| (row.sc_playlist_id, None)))
        .collect())
}
