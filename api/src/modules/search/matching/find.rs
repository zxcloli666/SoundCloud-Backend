use super::pick;
use super::wanted::Wanted;
use crate::error::AppResult;
use crate::modules::search::SearchService;

const COMBINED_LIMIT: i64 = 5;
const TITLE_LIMIT: i64 = 20;

pub async fn find(search: &SearchService, wanted: &Wanted) -> AppResult<Option<String>> {
    if wanted.title_key().is_empty() {
        return Ok(None);
    }
    let query = format!("{} {}", wanted.artist, wanted.title);
    let hits = search
        .tracks(query.trim(), None, 0, COMBINED_LIMIT)
        .await?
        .collection;
    if let Some(urn) = pick::combined(wanted, &hits) {
        return Ok(Some(urn));
    }
    if wanted.artist_key().is_empty() {
        return Ok(None);
    }
    let hits = search
        .tracks(&wanted.title, None, 0, TITLE_LIMIT)
        .await?
        .collection;
    Ok(pick::by_uploader(wanted, &hits))
}
