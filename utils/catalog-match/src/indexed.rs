use catalog_normalize::{TitleForms, normalize_title, title_forms, works_match};
use sqlx::PgPool;
use uuid::Uuid;

use crate::score::title_score;

pub const WORK_TITLE_THRESHOLD: f32 = 0.85;

const CANDIDATE_LIMIT: i64 = 200;
const SAME_RECORDING_SCORE: f32 = 1.0;
const SAME_WORK_SCORE: f32 = 0.9;

#[derive(Debug, Clone)]
pub struct IndexedMatch {
    pub track_id: Uuid,
    pub sc_track_id: String,
    pub score: f32,
}

pub async fn best_indexed_for_artist_title(
    pool: &PgPool,
    artist_id: Uuid,
    target_title: &str,
) -> Result<Option<IndexedMatch>, sqlx::Error> {
    let target = title_forms(target_title);
    let normalized = normalize_title(target_title);
    if target.work_key.is_empty() && normalized.is_empty() {
        return Ok(None);
    }
    let work_keys: Vec<String> = target.work_keys().map(str::to_owned).collect();
    let first_word_prefix = match normalized.split_whitespace().next() {
        Some(word) => format!("{word}%"),
        None => format!("{normalized}%"),
    };

    let candidates = sqlx::query_file!(
        "queries/indexed_candidates.sql",
        artist_id,
        &work_keys,
        &normalized,
        &first_word_prefix,
        CANDIDATE_LIMIT
    )
    .fetch_all(pool)
    .await?;

    let mut best: Option<IndexedMatch> = None;
    for candidate in candidates {
        if candidate.title.is_empty() {
            continue;
        }
        let score = score_against(&target, target_title, &candidate.title);
        if score < WORK_TITLE_THRESHOLD {
            continue;
        }
        if best.as_ref().is_none_or(|current| score > current.score) {
            best = Some(IndexedMatch {
                track_id: candidate.id,
                sc_track_id: candidate.sc_track_id,
                score,
            });
        }
    }
    Ok(best)
}

fn score_against(target: &TitleForms, target_title: &str, candidate_title: &str) -> f32 {
    let candidate = title_forms(candidate_title);
    if target.is_same_recording(&candidate) {
        return SAME_RECORDING_SCORE;
    }
    if works_match(target, &candidate) {
        return SAME_WORK_SCORE;
    }
    title_score(target_title, candidate_title, None)
}

pub async fn attach_genius_song(
    pool: &PgPool,
    track_id: Uuid,
    genius_song_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_file_scalar!(
        "queries/set_track_genius_song.sql",
        track_id,
        genius_song_id
    )
    .fetch_optional(pool)
    .await
}
