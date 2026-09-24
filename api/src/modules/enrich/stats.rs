use serde::Serialize;
use sqlx::PgPool;

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
pub struct EnrichStats {
    pub pending: i64,
    pub done: i64,
    pub failed: i64,
    pub dead: i64,
    pub in_flight: i64,
    pub artists: i64,
    pub albums: i64,
    pub crawl: CrawlStats,
    pub wanted: WantedStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrawlStats {
    pub artists_total: i64,
    pub genius_total: i64,
    pub genius_crawled: i64,
    pub mb_total: i64,
    pub mb_crawled: i64,
    pub due_now: i64,
    pub dead: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WantedStats {
    pub wanted: i64,
    pub unresolvable: i64,
}

pub async fn stats(pg: &PgPool) -> AppResult<EnrichStats> {
    let row = sqlx::query_file!("queries/enrich/service/stats_tracks.sql")
        .fetch_one(pg)
        .await?;
    let albums = sqlx::query_file_scalar!("queries/enrich/service/stats_albums.sql")
        .fetch_one(pg)
        .await?;
    let c = sqlx::query_file!("queries/enrich/service/stats_crawl.sql")
        .fetch_one(pg)
        .await?;
    let w = sqlx::query_file!("queries/enrich/service/stats_wanted.sql")
        .fetch_one(pg)
        .await?;
    Ok(EnrichStats {
        pending: row.pending,
        done: row.done,
        failed: row.failed,
        dead: row.dead,
        in_flight: row.in_flight,
        artists: c.artists_total,
        albums,
        crawl: CrawlStats {
            artists_total: c.artists_total,
            genius_total: c.genius_total,
            genius_crawled: c.genius_crawled,
            mb_total: c.mb_total,
            mb_crawled: c.mb_crawled,
            due_now: c.due_now,
            dead: c.dead,
        },
        wanted: WantedStats {
            wanted: w.wanted,
            unresolvable: w.unresolvable,
        },
    })
}
