use sqlx::PgPool;
use uuid::Uuid;

use super::error::EnrichResult;

pub async fn recompute_for_track(pg: &PgPool, track_id: Uuid) -> EnrichResult<()> {
    sqlx::query_file!("queries/enrich/coplay/recompute_for_track.sql", track_id)
        .execute(pg)
        .await?;
    Ok(())
}
