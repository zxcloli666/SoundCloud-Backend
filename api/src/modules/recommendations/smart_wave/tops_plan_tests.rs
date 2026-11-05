use serde_json::{Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

const WAVE_TOPS: &str =
    include_str!("../../../../queries/recommendations/smart_wave/graph/load_artist_top_tracks.sql");
const HOME_TOPS: &str =
    include_str!("../../../../queries/recommendations/home_wave/load_artist_top_tracks.sql");
const ARTISTS_READ: i64 = 160;
const TRACKS_KEPT: i64 = 64;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn wave_and_shelf_tops_read_tracks_only_through_their_partial_index_and_stop_at_the_cut(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let artists = sqlx::query_scalar::<_, Uuid>("SELECT id FROM artists ORDER BY id LIMIT $1")
        .bind(ARTISTS_READ)
        .fetch_all(&pool)
        .await?;
    for (sql, index) in [
        (WAVE_TOPS, "tracks_playable_idx"),
        (HOME_TOPS, "tracks_stored_idx"),
    ] {
        let explain = format!("EXPLAIN (FORMAT JSON) {sql}");
        let plan = sqlx::query_scalar::<_, Value>(&explain)
            .bind(&artists)
            .bind(TRACKS_KEPT)
            .fetch_one(&pool)
            .await?;
        let nodes = nodes(&plan);
        let tracks: Vec<_> = nodes
            .iter()
            .filter(|node| text(node, "Relation Name") == Some("tracks"))
            .collect();
        assert!(!tracks.is_empty(), "{index}: tracks are not read: {plan}");
        assert!(
            tracks.iter().all(|node| {
                text(node, "Node Type") == Some("Index Only Scan")
                    && text(node, "Index Name") == Some(index)
            }),
            "{index}: tracks are read past it: {plan}"
        );
        assert!(
            nodes.iter().any(|node| {
                text(node, "Node Type") == Some("WindowAgg") && node.contains_key("Run Condition")
            }),
            "{index}: the row number has no run condition: {plan}"
        );
    }
    Ok(())
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    crate::db::insert_replicated(
        pool,
        "WITH plan AS (
             SELECT a,
                    ('00000000-0000-4000-8001-' || lpad(to_hex(a), 12, '0'))::uuid AS artist_id,
                    sum(40 + a % 7 * 30) OVER (ORDER BY a) - (40 + a % 7 * 30) AS first,
                    40 + a % 7 * 30 AS count
             FROM generate_series(1, 300) a
         ),
         named AS (
             INSERT INTO artists (id, name, normalized_name, source)
             SELECT artist_id, artist_id::text, artist_id::text, 'test' FROM plan
         ),
         made AS (
             INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing,
                                 storage_state, index_state, needs_duration_resolve)
             SELECT n::text, 'urn:' || n, 'T' || n, 't' || n, 1000,
                    CASE WHEN n % 25 = 0 THEN 'private' ELSE 'public' END,
                    CASE WHEN n % 10 = 1 THEN 'pending' ELSE 'ok' END,
                    CASE WHEN n % 8 = 0 THEN 'indexed' ELSE 'pending' END,
                    n % 10 IN (2, 3, 4)
             FROM plan p
                      CROSS JOIN LATERAL generate_series(p.first + 1, p.first + p.count) AS n
             RETURNING id, sc_track_id
         ),
         credited AS (
             INSERT INTO track_artists (track_id, artist_id, role, source)
             SELECT m.id, p.artist_id, 'primary', 'test'
             FROM made m
                      JOIN plan p ON m.sc_track_id::bigint BETWEEN p.first + 1 AND p.first + p.count
         ),
         featured AS (
             INSERT INTO track_artists (track_id, artist_id, role, source)
             SELECT m.id, p.artist_id, 'featured', 'test'
             FROM made m
                      JOIN plan p ON p.a = 1 + m.sc_track_id::bigint * 13 % 300
             WHERE m.sc_track_id::bigint % 5 = 0
         )
         INSERT INTO sc_track_counters (sc_track_id, play_count)
         SELECT sc_track_id, sc_track_id::bigint * 7919 % 100003 FROM made",
    )
    .await?;
    sqlx::query("VACUUM ANALYZE artists, tracks, track_artists, sc_track_counters")
        .execute(pool)
        .await?;
    Ok(())
}

fn nodes(plan: &Value) -> Vec<&Map<String, Value>> {
    let mut found = Vec::new();
    let mut stack = vec![plan];
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(fields) => {
                if fields.contains_key("Node Type") {
                    found.push(fields);
                }
                stack.extend(fields.values());
            }
            _ => {}
        }
    }
    found
}

fn text<'a>(node: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    node.get(key).and_then(Value::as_str)
}
