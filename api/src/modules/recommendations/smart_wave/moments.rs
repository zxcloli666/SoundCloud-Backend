use std::collections::HashSet;

use sqlx::PgPool;
use tracing::debug;

use super::cursor::WaveMode;

const OVERFETCH: usize = 2;

pub fn widen(mode: WaveMode, pool: usize) -> usize {
    match mode {
        WaveMode::Full => pool,
        WaveMode::Moments => pool * OVERFETCH,
    }
}

pub async fn with_moment(pg: &PgPool, mode: WaveMode, ids: &[u64]) -> Option<HashSet<u64>> {
    if mode == WaveMode::Full {
        return None;
    }
    if ids.is_empty() {
        return Some(HashSet::new());
    }
    Some(highlighted(pg, ids).await)
}

pub fn allowed(found: Option<&HashSet<u64>>, sc_track_id: u64) -> bool {
    found.is_none_or(|found| found.contains(&sc_track_id))
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
