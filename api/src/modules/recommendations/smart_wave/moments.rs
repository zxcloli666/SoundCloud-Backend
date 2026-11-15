use std::collections::HashSet;

use sqlx::PgPool;
use tracing::debug;

use super::cursor::WaveMode;
use super::rank::Candidate;

const OVERFETCH: usize = 2;

pub fn widen(mode: WaveMode, pool: usize) -> usize {
    match mode {
        WaveMode::Full => pool,
        WaveMode::Moments => pool * OVERFETCH,
    }
}

pub async fn keep_with_moment(pg: &PgPool, mode: WaveMode, cands: &mut Vec<Candidate>) {
    if mode == WaveMode::Full || cands.is_empty() {
        return;
    }
    let ids: Vec<u64> = cands.iter().map(|cand| cand.sc_track_id).collect();
    let found = highlighted(pg, &ids).await;
    cands.retain(|cand| found.contains(&cand.sc_track_id));
}

async fn highlighted(pg: &PgPool, ids: &[u64]) -> HashSet<u64> {
    let keys: Vec<String> = ids.iter().map(u64::to_string).collect();
    match sqlx::query_file_scalar!("queries/recommendations/smart_wave/highlighted.sql", &keys)
        .fetch_all(pg)
        .await
    {
        Ok(rows) => rows.iter().filter_map(|id| id.parse().ok()).collect(),
        Err(error) => {
            debug!(%error, "wave: moments lookup failed, the batch has no moments");
            HashSet::new()
        }
    }
}
