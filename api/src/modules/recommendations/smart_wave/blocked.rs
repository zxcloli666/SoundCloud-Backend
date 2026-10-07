use std::collections::HashSet;

use sqlx::PgPool;

pub async fn blocked_uploads(pg: &PgPool, variants: &[String], ids: &[u64]) -> HashSet<u64> {
    if ids.is_empty() || variants.is_empty() {
        return HashSet::new();
    }
    let id_strs: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    sqlx::query_file_scalar!(
        "queries/recommendations/smart_wave/blocked/load_blocked_uploads.sql",
        &id_strs,
        variants
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|id| id.parse::<u64>().ok())
    .collect()
}
