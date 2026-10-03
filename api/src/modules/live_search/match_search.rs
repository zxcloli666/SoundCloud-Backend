use axum::http::StatusCode;
use serde_json::Value;
use sqlx::PgPool;
use tracing::debug;

use super::matching::{Judged, MatchReply, Wanted, judge};
use super::merge::{SOURCE_LOCAL, SOURCE_SOUNDCLOUD, is_live};
use super::meta::{LiveMeta, LiveState};
use super::service::LiveSearch;
use crate::error::{AppError, AppResult};
use crate::modules::search::query::TrackSearchQuery;
use crate::modules::search::terms::QueryTerms;
use crate::modules::search::{SearchService, candidates, failure};

const LOCAL_POOL: i64 = 20;
const PACED_RETRY_AFTER: i64 = 1;

pub async fn find(
    search: &SearchService,
    live: &LiveSearch,
    pg: &PgPool,
    wanted: &Wanted,
    sc_user_id: &str,
) -> AppResult<MatchReply> {
    let local = local_matches(search, pg, wanted).await;
    let known = match &local {
        Ok(judged) => judged.clone(),
        Err(error) => {
            debug!(%error, "local match candidates are unavailable");
            Vec::new()
        }
    };
    let settled = MatchReply::decide(known.clone(), LiveMeta::new(LiveState::Local, None));
    if settled.found.is_some() {
        return Ok(settled);
    }
    let Some(request) = live.match_plan(&wanted.query(), sc_user_id) else {
        return local.map(|_| settled);
    };
    let (hits, meta) = live.hits(&request).await;
    if hits.is_empty() && matches!(meta.state, LiveState::Limited | LiveState::Busy) {
        return Err(paced(meta.retry_after_sec));
    }
    let meta = match local {
        Err(error) if hits.is_empty() => return Err(error),
        Err(_) => meta.local_unavailable(),
        Ok(_) => meta,
    };
    let mut judged = known;
    judged.extend(hits.into_iter().filter_map(|item| {
        let source = if is_live(&item) {
            SOURCE_SOUNDCLOUD
        } else {
            SOURCE_LOCAL
        };
        judge(wanted, item, source, None)
    }));
    let reply = MatchReply::decide(judged, meta);
    if let Some(found) = reply
        .found
        .as_ref()
        .filter(|found| found.source == SOURCE_SOUNDCLOUD)
    {
        live.keep_pick(&found.item).await;
    }
    Ok(reply)
}

async fn local_matches(
    search: &SearchService,
    pg: &PgPool,
    wanted: &Wanted,
) -> AppResult<Vec<Judged>> {
    let query = TrackSearchQuery {
        q: Some(wanted.query()),
        ..Default::default()
    };
    let (indexed, pool) = tokio::join!(indexed(pg, wanted), search.tracks(&query, 0, LOCAL_POOL));
    let mut judged: Vec<Judged> = indexed?
        .into_iter()
        .filter_map(|(item, score)| judge(wanted, item, SOURCE_LOCAL, Some(score)))
        .collect();
    judged.extend(
        pool?
            .collection
            .into_iter()
            .filter_map(|item| judge(wanted, item, SOURCE_LOCAL, None)),
    );
    Ok(judged)
}

async fn indexed(pg: &PgPool, wanted: &Wanted) -> AppResult<Vec<(Value, f32)>> {
    let artist = QueryTerms::parse(&wanted.artist);
    if artist.norm.is_empty() {
        return Ok(Vec::new());
    }
    let mut names = vec![artist.norm.clone()];
    names.extend(artist.latin().map(|latin| latin.norm));
    let mut found = Vec::new();
    for artist_id in candidates::span_artists(pg, &names).await? {
        let best = catalog_match::best_indexed_for_artist_title(pg, artist_id, &wanted.title)
            .await
            .map_err(failure::from_db)?;
        found.extend(best);
    }
    let ids: Vec<String> = found.iter().map(|best| best.sc_track_id.clone()).collect();
    let items = crate::modules::tracks::project_many_public(pg, &ids).await?;
    Ok(items
        .into_iter()
        .zip(found)
        .filter_map(|(item, best)| Some((item?, best.score)))
        .collect())
}

fn paced(retry_after: Option<i64>) -> AppError {
    AppError::coded(
        StatusCode::TOO_MANY_REQUESTS,
        "search_paced",
        "Too many tracks matched at once, retry shortly",
    )
    .with_retry_after(retry_after.unwrap_or(PACED_RETRY_AFTER))
}
