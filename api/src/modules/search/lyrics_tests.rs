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
    super::lexicon_refresh::refresh(pool).await
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

#[sqlx::test(migrations = "./migrations")]
async fn a_common_chorus_over_long_lyrics_stays_fast(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, play_count_sc)
         SELECT n::text, 'soundcloud:tracks:' || n, 'cover ' || n, 'cover ' || n, 200000, 'public', n
         FROM generate_series(1, 400) n;
         INSERT INTO lyrics_cache (sc_track_id, plain_text, source, plain_source)
         SELECT n::text,
                (SELECT string_agg(CASE WHEN k = 75 THEN 'forever young heart of glass'
                                        ELSE 'we keep on dancing through the neon night ' || k || ' baby ' || n END, E'\\n')
                 FROM generate_series(1, 150) k),
                'lrclib', 'lrclib'
         FROM generate_series(1, 400) n",
    )
    .execute(&pool)
    .await?;
    super::lexicon_refresh::refresh(&pool).await?;
    sqlx::raw_sql("ANALYZE tracks, lyrics_cache, search_terms")
        .execute(&pool)
        .await?;
    let search = service(&pool)?;
    for page in [0, 1] {
        let started = std::time::Instant::now();
        let answer = search
            .lyrics("forever yung hart of glas", Some(page), Some(50))
            .await?;
        let elapsed = started.elapsed();
        assert!(elapsed.as_millis() < 750, "page {page} took {elapsed:?}");
        assert_eq!(answer.collection.len(), 50);
        assert!(
            answer
                .collection
                .iter()
                .all(|hit| hit.matched_line.as_deref() == Some("forever young heart of glass")),
            "page {page}"
        );
    }
    Ok(())
}
