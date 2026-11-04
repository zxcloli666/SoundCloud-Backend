use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

const OLD_WAVE_TOPS: &str = "WITH ranked AS (SELECT ta.artist_id,
                       it.sc_track_id,
                       COALESCE(c.play_count, 0) AS play_count,
                       ROW_NUMBER() OVER (
            PARTITION BY ta.artist_id ORDER BY COALESCE(c.play_count, 0) DESC, it.sc_track_id
        ) AS rn,
                       COUNT(*) OVER (PARTITION BY ta.artist_id) AS total
                FROM track_artists ta
                         JOIN tracks it ON it.id = ta.track_id
                         LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                WHERE ta.artist_id = ANY ($1)
                  AND ta.role = 'primary'
                  AND it.sharing = 'public'
                  AND it.superseded_by IS NULL
                  AND it.storage_state = 'ok'
                  AND it.index_state = 'indexed')
SELECT artist_id, sc_track_id, play_count, (total > $2)
FROM ranked
WHERE rn <= $2
ORDER BY artist_id, rn";

const OLD_HOME_TOPS: &str = "WITH ranked AS (SELECT ta.artist_id,
                       it.sc_track_id,
                       COALESCE(c.play_count, 0) AS play_count,
                       ROW_NUMBER() OVER (
            PARTITION BY ta.artist_id ORDER BY COALESCE(c.play_count, 0) DESC, it.sc_track_id
        ) AS rn,
                       COUNT(*) OVER (PARTITION BY ta.artist_id) AS total
                FROM track_artists ta
                         JOIN tracks it ON it.id = ta.track_id
                         LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                WHERE ta.artist_id = ANY ($1)
                  AND ta.role = 'primary'
                  AND it.sharing = 'public'
                  AND it.superseded_by IS NULL
                  AND it.storage_state = 'ok'
                  AND it.needs_duration_resolve = false)
SELECT artist_id, sc_track_id, play_count, (total > $2)
FROM ranked
WHERE rn <= $2
ORDER BY artist_id, rn";

const OLD_COLLECT: &str = "WITH ranked AS (SELECT ta.artist_id,
                       it.sc_track_id,
                       ROW_NUMBER() OVER (
            PARTITION BY ta.artist_id ORDER BY COALESCE(c.play_count, 0) DESC
        ) AS rn
                FROM track_artists ta
                         JOIN tracks it ON it.id = ta.track_id
                         LEFT JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
                WHERE ta.artist_id = ANY ($1)
                  AND ta.role = 'primary'
                  AND it.sharing = 'public'
                  AND it.superseded_by IS NULL
                  AND it.storage_state = 'ok'
                  AND it.index_state = 'indexed'
                  AND NOT (it.sc_track_id = ANY ($2)))
SELECT artist_id, sc_track_id
FROM ranked
WHERE rn <= $3 LIMIT $4";

const HUB: Uuid = Uuid::from_u128(10);
const FEW: Uuid = Uuid::from_u128(11);
const EDGE: Uuid = Uuid::from_u128(12);
const NONE: Uuid = Uuid::from_u128(13);
const GUEST: Uuid = Uuid::from_u128(14);
const UNKNOWN: Uuid = Uuid::from_u128(99);
const LIMITS: [i64; 6] = [1, 5, 30, 64, 65, 500];

type Top = (Uuid, String, i64, bool);

fn inputs() -> Vec<Vec<Uuid>> {
    vec![
        vec![HUB],
        vec![FEW],
        vec![EDGE],
        vec![HUB, FEW, EDGE, NONE, GUEST],
        vec![EDGE, HUB, EDGE, FEW, HUB],
        vec![UNKNOWN, NONE],
        Vec::new(),
    ]
}

async fn tracks(
    pool: &PgPool,
    artist: Uuid,
    first: i64,
    count: i64,
    mixed: bool,
) -> anyhow::Result<()> {
    let last = first + count - 1;
    crate::db::insert_replicated(
        pool,
        &format!(
            "WITH made AS (
                 INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                                     sharing, storage_state, index_state, needs_duration_resolve)
                 SELECT n::text, 'urn:' || n, 'T' || n, 't' || n, 1000,
                        CASE WHEN {mixed} AND n % 11 = 0 THEN 'private' ELSE 'public' END,
                        CASE WHEN {mixed} AND n % 4 = 0 THEN 'pending' ELSE 'ok' END,
                        CASE WHEN {mixed} AND n % 6 = 0 THEN 'pending' ELSE 'indexed' END,
                        {mixed} AND n % 9 = 0
                 FROM generate_series({first}, {last}) AS n
                 RETURNING id, sc_track_id
             ), credited AS (
                 INSERT INTO track_artists (track_id, artist_id, role, source)
                 SELECT id, '{artist}'::uuid, 'primary', 'test' FROM made
             )
             INSERT INTO sc_track_counters (sc_track_id, play_count)
             SELECT sc_track_id,
                    CASE WHEN sc_track_id::bigint % 17 = 0 THEN 777
                         ELSE (sc_track_id::bigint * 7919) % 100003 END
             FROM made
             WHERE sc_track_id::bigint % 5 <> 0"
        ),
    )
    .await
}

async fn seed(pool: &PgPool) -> anyhow::Result<HashMap<String, i64>> {
    for (artist, name) in [
        (HUB, "hub"),
        (FEW, "few"),
        (EDGE, "edge"),
        (NONE, "none"),
        (GUEST, "guest"),
    ] {
        sqlx::query(
            "INSERT INTO artists (id, name, normalized_name, source) VALUES ($1, $2, $2, 'test')",
        )
        .bind(artist)
        .bind(name)
        .execute(pool)
        .await?;
    }
    tracks(pool, HUB, 1000, 400, true).await?;
    tracks(pool, FEW, 3000, 30, false).await?;
    tracks(pool, EDGE, 4000, 65, false).await?;
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source)
         SELECT ta.track_id, a.id, a.role, 'test'
         FROM track_artists ta
         CROSS JOIN (VALUES ($1::uuid, 'primary'), ($2::uuid, 'featured')) AS a(id, role)
         WHERE ta.artist_id = $3 AND ta.role = 'primary'
         ORDER BY ta.track_id
         LIMIT 120",
    )
    .bind(FEW)
    .bind(GUEST)
    .bind(HUB)
    .execute(pool)
    .await?;
    sqlx::query("VACUUM ANALYZE tracks, track_artists, sc_track_counters")
        .execute(pool)
        .await?;
    let plays: Vec<(String, i64)> =
        sqlx::query_as("SELECT sc_track_id, play_count FROM sc_track_counters")
            .fetch_all(pool)
            .await?;
    Ok(plays.into_iter().collect())
}

async fn old_tops(pool: &PgPool, sql: &str, artists: &[Uuid], k: i64) -> anyhow::Result<Vec<Top>> {
    Ok(sqlx::query_as(sql)
        .bind(artists)
        .bind(k)
        .fetch_all(pool)
        .await?)
}

async fn new_wave_tops(pool: &PgPool, artists: &[Uuid], k: i64) -> anyhow::Result<Vec<Top>> {
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_artist_top_tracks.sql",
        artists,
        k
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.artist_id,
                row.sc_track_id,
                row.play_count,
                row.truncated,
            )
        })
        .collect())
}

async fn new_home_tops(pool: &PgPool, artists: &[Uuid], k: i64) -> anyhow::Result<Vec<Top>> {
    let rows = sqlx::query_file!(
        "queries/recommendations/home_wave/load_artist_top_tracks.sql",
        artists,
        k
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.artist_id,
                row.sc_track_id,
                row.play_count,
                row.truncated,
            )
        })
        .collect())
}

async fn old_collect(
    pool: &PgPool,
    artists: &[Uuid],
    exclude: &[String],
    per_artist: i64,
    total: i64,
) -> anyhow::Result<Vec<(Uuid, String)>> {
    Ok(sqlx::query_as(OLD_COLLECT)
        .bind(artists)
        .bind(exclude)
        .bind(per_artist)
        .bind(total)
        .fetch_all(pool)
        .await?)
}

async fn new_collect(
    pool: &PgPool,
    artists: &[Uuid],
    exclude: &[String],
    per_artist: i64,
    total: i64,
) -> anyhow::Result<Vec<(Uuid, String)>> {
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/collect_artist_tracks.sql",
        artists,
        exclude,
        per_artist,
        total
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.artist_id, row.sc_track_id))
        .collect())
}

fn with_plays(rows: &[(Uuid, String)], plays: &HashMap<String, i64>) -> Vec<(Uuid, i64)> {
    rows.iter()
        .map(|(artist, id)| (*artist, plays.get(id).copied().unwrap_or(0)))
        .collect()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn per_artist_top_tracks_return_the_old_rows_for_every_input_and_limit(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    for artists in inputs() {
        for k in LIMITS {
            let old = old_tops(&pool, OLD_WAVE_TOPS, &artists, k).await?;
            assert_eq!(
                new_wave_tops(&pool, &artists, k).await?,
                old,
                "wave {artists:?} {k}"
            );
            let old = old_tops(&pool, OLD_HOME_TOPS, &artists, k).await?;
            assert_eq!(
                new_home_tops(&pool, &artists, k).await?,
                old,
                "home {artists:?} {k}"
            );
        }
    }
    let edge = new_wave_tops(&pool, &[EDGE], 64).await?;
    assert_eq!(edge.len(), 64);
    assert!(edge.iter().all(|row| row.3));
    assert!(
        new_wave_tops(&pool, &[EDGE], 65)
            .await?
            .iter()
            .all(|row| !row.3)
    );
    let hub = new_home_tops(&pool, &[HUB], 500).await?;
    let wave = new_wave_tops(&pool, &[HUB], 500).await?;
    assert_ne!(hub.len(), wave.len());
    assert!(wave.windows(2).any(|pair| pair[0].2 == pair[1].2));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn per_artist_collect_keeps_the_old_artists_and_play_counts(
    pool: PgPool,
) -> anyhow::Result<()> {
    let plays = seed(&pool).await?;
    let excluded: Vec<String> = (1000..1400)
        .step_by(7)
        .chain(3000..3010)
        .map(|n: i64| n.to_string())
        .collect();
    for artists in inputs() {
        for exclude in [&Vec::new(), &excluded] {
            for (per_artist, total) in [(6, 800), (3, 5), (12, 800), (6, 0), (64, 40), (500, 900)] {
                let old = old_collect(&pool, &artists, exclude, per_artist, total).await?;
                let new = new_collect(&pool, &artists, exclude, per_artist, total).await?;
                assert_eq!(
                    with_plays(&new, &plays),
                    with_plays(&old, &plays),
                    "{artists:?} {per_artist} {total}"
                );
            }
        }
    }
    Ok(())
}
