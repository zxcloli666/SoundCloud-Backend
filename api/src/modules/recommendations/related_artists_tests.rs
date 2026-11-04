use std::collections::{HashMap, HashSet};

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const OLD_WAVE_RELATED: &str = "SELECT a.id, a.name, a.avatar_url
FROM artist_coplay ac
         JOIN artists a ON a.id = CASE WHEN ac.a_id = $1 THEN ac.b_id ELSE ac.a_id END
WHERE (ac.a_id = $1 OR ac.b_id = $1)
  AND a.merged_into IS NULL
ORDER BY ac.weight DESC LIMIT $2";

const OLD_DETAIL_RELATED: &str = "SELECT a.id, a.name, a.country, a.avatar_url, ac.weight
FROM artist_coplay ac
         JOIN artists a ON a.id = CASE WHEN ac.a_id = $1 THEN ac.b_id ELSE ac.a_id END
WHERE (ac.a_id = $1 OR ac.b_id = $1)
  AND a.merged_into IS NULL
ORDER BY ac.weight DESC LIMIT 12";

const NEW_WAVE_RELATED: &str =
    include_str!("../../../queries/recommendations/artist_wave/load_related_artists.sql");
const NEW_DETAIL_RELATED: &str =
    include_str!("../../../queries/artists/handlers/detail_related.sql");

const HUB: u128 = 1_000_000;
const SURVIVOR: u128 = 5_000_000;
const LONELY: u128 = 5_000_001;
const SIDE: u128 = 150;
const BIG_SIDE: u128 = 5_000;
const LIMITS: [i64; 8] = [1, 5, 12, 20, 40, 120, 299, 400];

type WaveRow = (Uuid, String, Option<String>);
type DetailRow = (Uuid, String, Option<String>, Option<String>, f32);

fn neighbors(side: u128) -> impl Iterator<Item = u128> {
    (HUB - side..HUB).chain(HUB + 1..=HUB + side)
}

fn merged(n: u128) -> bool {
    n.is_multiple_of(7)
}

fn distinct_weight(n: u128) -> f32 {
    ((n * 7919) % 10_007) as f32 + 1.0
}

fn tied_weight(n: u128) -> f32 {
    (n % 3 + 1) as f32
}

async fn seed(
    pool: &PgPool,
    side: u128,
    weight: fn(u128) -> f32,
) -> anyhow::Result<HashMap<Uuid, f32>> {
    let ids: Vec<Uuid> = neighbors(side)
        .chain([HUB, SURVIVOR, LONELY])
        .map(Uuid::from_u128)
        .collect();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, country, avatar_url)
         SELECT id, 'artist ' || id, id::text, 'test',
                CASE WHEN n % 2 = 1 THEN 'DE' END,
                CASE WHEN n % 2 = 0 THEN 'https://img/' || id END
         FROM unnest($1::uuid[]) WITH ORDINALITY AS u(id, n)",
    )
    .bind(&ids)
    .execute(pool)
    .await?;
    let (others, weights): (Vec<Uuid>, Vec<f32>) = neighbors(side)
        .map(|n| (Uuid::from_u128(n), weight(n)))
        .unzip();
    sqlx::query(
        "INSERT INTO artist_coplay (a_id, b_id, weight)
         SELECT LEAST($1, other), GREATEST($1, other), w
         FROM unnest($2::uuid[], $3::real[]) AS e(other, w)",
    )
    .bind(Uuid::from_u128(HUB))
    .bind(&others)
    .bind(&weights)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO artist_coplay (a_id, b_id, weight) VALUES ($1, $2, 99999)")
        .bind(Uuid::from_u128(SURVIVOR))
        .bind(Uuid::from_u128(LONELY))
        .execute(pool)
        .await?;
    let gone: Vec<Uuid> = neighbors(side)
        .filter(|n| merged(*n))
        .map(Uuid::from_u128)
        .collect();
    sqlx::query("UPDATE artists SET merged_into = $1 WHERE id = ANY ($2)")
        .bind(Uuid::from_u128(SURVIVOR))
        .bind(&gone)
        .execute(pool)
        .await?;
    sqlx::query("VACUUM ANALYZE artists, artist_coplay")
        .execute(pool)
        .await?;
    Ok(others.into_iter().zip(weights).collect())
}

async fn old_wave(pool: &PgPool, artist: Uuid, limit: i64) -> anyhow::Result<Vec<WaveRow>> {
    Ok(sqlx::query_as(OLD_WAVE_RELATED)
        .bind(artist)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

async fn new_wave(pool: &PgPool, artist: Uuid, limit: i64) -> anyhow::Result<Vec<WaveRow>> {
    let rows = sqlx::query_file!(
        "queries/recommendations/artist_wave/load_related_artists.sql",
        artist,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.name, row.avatar_url))
        .collect())
}

async fn old_detail(pool: &PgPool, artist: Uuid) -> anyhow::Result<Vec<DetailRow>> {
    Ok(sqlx::query_as(OLD_DETAIL_RELATED)
        .bind(artist)
        .fetch_all(pool)
        .await?)
}

async fn new_detail(pool: &PgPool, artist: Uuid) -> anyhow::Result<Vec<DetailRow>> {
    let rows = sqlx::query_file!("queries/artists/handlers/detail_related.sql", artist)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.name, row.country, row.avatar_url, row.weight))
        .collect())
}

fn weights_of(rows: &[WaveRow], weight: &HashMap<Uuid, f32>) -> Vec<f32> {
    rows.iter().map(|row| weight[&row.0]).collect()
}

fn above(rows: &[WaveRow], weight: &HashMap<Uuid, f32>, cut: f32) -> HashSet<Uuid> {
    rows.iter()
        .map(|row| row.0)
        .filter(|id| weight[id] > cut)
        .collect()
}

async fn coplay_rows_read(pool: &PgPool, sql: &str, limit: Option<i64>) -> anyhow::Result<f64> {
    let explain = format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}");
    let query = sqlx::query_scalar::<_, Value>(&explain).bind(Uuid::from_u128(HUB));
    let plan = match limit {
        Some(limit) => query.bind(limit).fetch_one(pool).await?,
        None => query.fetch_one(pool).await?,
    };
    let mut read = 0.0;
    let mut stack = vec![&plan];
    while let Some(node) = stack.pop() {
        match node {
            Value::Array(items) => stack.extend(items),
            Value::Object(fields) => {
                if fields.get("Relation Name").and_then(Value::as_str) == Some("artist_coplay") {
                    let rows = fields.get("Actual Rows").and_then(Value::as_f64);
                    let loops = fields.get("Actual Loops").and_then(Value::as_f64);
                    read += rows.unwrap_or(0.0) * loops.unwrap_or(0.0);
                }
                stack.extend(fields.values());
            }
            _ => {}
        }
    }
    Ok(read)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn bounded_related_artists_match_the_full_sort_for_every_limit(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, SIDE, distinct_weight).await?;
    for artist in [HUB, SURVIVOR, LONELY, HUB - 1, HUB + SIDE, 77] {
        let artist = Uuid::from_u128(artist);
        for limit in LIMITS {
            let old = old_wave(&pool, artist, limit).await?;
            assert_eq!(
                new_wave(&pool, artist, limit).await?,
                old,
                "{artist} {limit}"
            );
        }
        assert_eq!(
            new_detail(&pool, artist).await?,
            old_detail(&pool, artist).await?
        );
    }
    let hub = Uuid::from_u128(HUB);
    let alive = neighbors(SIDE).filter(|n| !merged(*n)).count();
    assert_eq!(new_wave(&pool, hub, 400).await?.len(), alive);
    let detail = new_detail(&pool, hub).await?;
    assert_eq!(detail.len(), 12);
    assert!(detail.iter().any(|row| row.2.is_some()));
    assert!(detail.iter().any(|row| row.3.is_some()));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn tied_weights_keep_the_same_weights_and_the_same_artists_above_the_cut(
    pool: PgPool,
) -> anyhow::Result<()> {
    let weight = seed(&pool, SIDE, tied_weight).await?;
    let hub = Uuid::from_u128(HUB);
    for limit in LIMITS {
        let old = old_wave(&pool, hub, limit).await?;
        let new = new_wave(&pool, hub, limit).await?;
        assert_eq!(
            weights_of(&new, &weight),
            weights_of(&old, &weight),
            "{limit}"
        );
        let cut = weights_of(&old, &weight).last().copied().unwrap_or(0.0);
        assert_eq!(
            above(&new, &weight, cut),
            above(&old, &weight, cut),
            "{limit}"
        );
        let unique: HashSet<Uuid> = new.iter().map(|row| row.0).collect();
        assert_eq!(unique.len(), new.len());
    }
    let old: Vec<f32> = old_detail(&pool, hub).await?.iter().map(|r| r.4).collect();
    let new: Vec<f32> = new_detail(&pool, hub).await?.iter().map(|r| r.4).collect();
    assert_eq!(new, old);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn related_artists_of_a_hub_read_a_few_edges_instead_of_all_of_them(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, BIG_SIDE, distinct_weight).await?;
    let hub = Uuid::from_u128(HUB);
    assert_eq!(
        new_wave(&pool, hub, 20).await?,
        old_wave(&pool, hub, 20).await?
    );
    assert_eq!(new_detail(&pool, hub).await?, old_detail(&pool, hub).await?);
    let every = 2.0 * BIG_SIDE as f64;
    assert_eq!(
        coplay_rows_read(&pool, OLD_WAVE_RELATED, Some(20)).await?,
        every
    );
    assert_eq!(
        coplay_rows_read(&pool, OLD_DETAIL_RELATED, None).await?,
        every
    );
    let wave = coplay_rows_read(&pool, NEW_WAVE_RELATED, Some(20)).await?;
    let detail = coplay_rows_read(&pool, NEW_DETAIL_RELATED, None).await?;
    assert!(wave <= 60.0, "the wave reads {wave} edges");
    assert!(detail <= 40.0, "the artist page reads {detail} edges");
    Ok(())
}
