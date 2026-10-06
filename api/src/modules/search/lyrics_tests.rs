use std::sync::Arc;

use sqlx::PgPool;

use super::SearchService;
use crate::cache::CacheService;

fn service(pool: &PgPool) -> anyhow::Result<Arc<SearchService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SearchService::new(pool.clone(), CacheService::new(redis)))
}

async fn song(
    pool: &PgPool,
    id: i64,
    title: &str,
    lyrics: &str,
    source: &str,
    plain_source: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, play_count_sc)
         VALUES ($1::bigint::text, 'soundcloud:tracks:' || $1, $2, lower($2), 200000, 'public', $1)",
    )
    .bind(id)
    .bind(title)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source, plain_source)
         VALUES ($1::bigint::text, $2, $3, $4)",
    )
    .bind(id)
    .bind(lyrics)
    .bind(source)
    .bind(plain_source)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    song(
        pool,
        1,
        "Nothing Else Matters",
        "So close no matter how far\nCouldn't be much more from the heart\nForever trusting who we are\nAnd nothing else matters",
        "lrclib",
        Some("lrclib"),
    )
    .await?;
    song(
        pool,
        2,
        "Decoy",
        "forever young\ntrusting nobody\nwho knows\nwe are the night\nheart of glass\nso far away",
        "lrclib",
        Some("lrclib"),
    )
    .await?;
    song(
        pool,
        3,
        "Gibberish",
        "forever trusting who we are heart forever trusting who we are",
        "self_gen",
        None,
    )
    .await?;
    song(
        pool,
        4,
        "Deleted",
        "Forever trusting who we are\nforever trusting",
        "lrclib",
        Some("lrclib"),
    )
    .await?;
    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '4'")
        .execute(pool)
        .await?;
    sqlx::query("REFRESH MATERIALIZED VIEW search_terms")
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_misremembered_line_finds_the_song_and_its_line(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let answer = service(&pool)?
        .lyrics("forevr trusting who we ar heart", None, None)
        .await?;
    assert_eq!(answer.mode, "text");
    let first = answer.collection.first().expect("a hit");
    assert_eq!(first.track["id"], 1);
    assert_eq!(
        first.matched_line.as_deref(),
        Some("Forever trusting who we are")
    );
    assert!((0.0..=1.0).contains(&first.score));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn gibberish_and_deleted_tracks_are_never_hits(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let answer = service(&pool)?
        .lyrics("forever trusting who we are", None, Some(50))
        .await?;
    let ids: Vec<i64> = answer
        .collection
        .iter()
        .filter_map(|hit| hit.track["id"].as_i64())
        .collect();
    assert!(ids.contains(&1), "{ids:?}");
    assert!(!ids.contains(&3) && !ids.contains(&4), "{ids:?}");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_short_query_is_an_empty_text_page(pool: PgPool) -> anyhow::Result<()> {
    let answer = service(&pool)?.lyrics(" a ", Some(-1), Some(500)).await?;
    assert_eq!(
        (
            answer.page,
            answer.page_size,
            answer.has_more,
            answer.mode.as_str()
        ),
        (0, 50, false, "text")
    );
    assert!(answer.collection.is_empty());
    Ok(())
}
