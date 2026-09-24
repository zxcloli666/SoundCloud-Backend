use sqlx::PgPool;
use uuid::Uuid;

use super::RecommendationHandler;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             merged_into uuid
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL UNIQUE,
             sharing varchar(8) NOT NULL DEFAULT 'public',
             storage_state varchar(16) NOT NULL DEFAULT 'pending',
             storage_priority smallint NOT NULL DEFAULT 5,
             index_priority smallint NOT NULL DEFAULT 5
         );
         CREATE TABLE track_artists (
             track_id uuid NOT NULL,
             artist_id uuid NOT NULL,
             role varchar(16) NOT NULL
         );
         CREATE TABLE user_likes_tracks (
             user_id text NOT NULL,
             sc_track_id text NOT NULL,
             wanted_state boolean NOT NULL DEFAULT true,
             created_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (user_id, sc_track_id)
         );
         CREATE TABLE artist_colike (
             a_id uuid NOT NULL,
             b_id uuid NOT NULL,
             co integer NOT NULL,
             w real NOT NULL,
             updated_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (a_id, b_id)
         );
         CREATE TABLE user_events (
             id uuid PRIMARY KEY,
             sc_user_id text NOT NULL,
             created_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE sc_track_counters (
             sc_track_id text PRIMARY KEY,
             play_count bigint
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_artist_track(
    pool: &PgPool,
    artist_id: Uuid,
    track_id: Uuid,
    sc_track_id: &str,
    storage_state: &str,
    priority: i16,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             id, sc_track_id, storage_state, storage_priority, index_priority
         ) VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(track_id)
    .bind(sc_track_id)
    .bind(storage_state)
    .bind(priority)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role)
         VALUES ($1, $2, 'primary')",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn colike_rebuild_inserts_shared_audience_edge(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let first_artist = Uuid::from_u128(1);
    let second_artist = Uuid::from_u128(2);
    sqlx::query("INSERT INTO artists (id) VALUES ($1), ($2)")
        .bind(first_artist)
        .bind(second_artist)
        .execute(&pool)
        .await?;
    insert_artist_track(&pool, first_artist, Uuid::from_u128(11), "track-1", "ok", 5).await?;
    insert_artist_track(
        &pool,
        second_artist,
        Uuid::from_u128(12),
        "track-2",
        "ok",
        5,
    )
    .await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id)
         VALUES
             ('100', 'track-1'),
             ('100', 'track-2'),
             ('200', 'track-1'),
             ('200', 'track-2')",
    )
    .execute(&pool)
    .await?;

    RecommendationHandler::new(pool.clone(), 1)
        .rebuild_colike()
        .await?;

    let edge: (i64, Option<i32>) = sqlx::query_as("SELECT count(*), min(co) FROM artist_colike")
        .fetch_one(&pool)
        .await?;
    assert_eq!(edge, (1, Some(2)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn colike_rebuild_prunes_edges_missing_from_latest_snapshot(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let first_artist = Uuid::from_u128(1);
    let second_artist = Uuid::from_u128(2);
    sqlx::query("INSERT INTO artists (id) VALUES ($1), ($2)")
        .bind(first_artist)
        .bind(second_artist)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO artist_colike (a_id, b_id, co, w, updated_at)
         VALUES ($1, $2, 3, 0.25, now() - interval '1 day')",
    )
    .bind(first_artist)
    .bind(second_artist)
    .execute(&pool)
    .await?;

    RecommendationHandler::new(pool.clone(), 1)
        .rebuild_colike()
        .await?;

    let edges: i64 = sqlx::query_scalar("SELECT count(*) FROM artist_colike")
        .fetch_one(&pool)
        .await?;
    assert_eq!(edges, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn wave_priority_promotes_four_most_played_tracks_per_artist(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = Uuid::from_u128(1);
    sqlx::query("INSERT INTO artists (id) VALUES ($1)")
        .bind(artist_id)
        .execute(&pool)
        .await?;
    insert_artist_track(&pool, artist_id, Uuid::from_u128(10), "seed-track", "ok", 5).await?;

    for rank in 1_u128..=5 {
        let sc_track_id = format!("candidate-{rank}");
        insert_artist_track(
            &pool,
            artist_id,
            Uuid::from_u128(10 + rank),
            &sc_track_id,
            "pending",
            5,
        )
        .await?;
        sqlx::query(
            "INSERT INTO sc_track_counters (sc_track_id, play_count)
             VALUES ($1, $2)",
        )
        .bind(sc_track_id)
        .bind(60_i64 - rank as i64 * 10)
        .execute(&pool)
        .await?;
    }

    sqlx::query(
        "INSERT INTO user_events (id, sc_user_id)
         VALUES ($1, '100'), ($2, 'soundcloud:users:100')",
    )
    .bind(Uuid::from_u128(100))
    .bind(Uuid::from_u128(101))
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id)
         VALUES ('100', 'seed-track')",
    )
    .execute(&pool)
    .await?;

    RecommendationHandler::new(pool.clone(), 1)
        .bump_wave_priority()
        .await?;

    let priorities: (i64, Option<i16>) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE storage_priority = 0),
                min(storage_priority) FILTER (WHERE sc_track_id = 'candidate-5')
         FROM tracks
         WHERE storage_state = 'pending'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(priorities, (4, Some(5)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn wave_priority_ignores_users_without_recent_playback(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = Uuid::from_u128(1);
    sqlx::query("INSERT INTO artists (id) VALUES ($1)")
        .bind(artist_id)
        .execute(&pool)
        .await?;
    insert_artist_track(&pool, artist_id, Uuid::from_u128(10), "seed-track", "ok", 5).await?;
    insert_artist_track(
        &pool,
        artist_id,
        Uuid::from_u128(11),
        "candidate",
        "pending",
        5,
    )
    .await?;
    sqlx::query(
        "INSERT INTO user_events (id, sc_user_id, created_at)
         VALUES ($1, '100', now() - interval '15 days')",
    )
    .bind(Uuid::from_u128(100))
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id)
         VALUES ('100', 'seed-track')",
    )
    .execute(&pool)
    .await?;

    RecommendationHandler::new(pool.clone(), 1)
        .bump_wave_priority()
        .await?;

    let priority: i16 =
        sqlx::query_scalar("SELECT storage_priority FROM tracks WHERE sc_track_id = 'candidate'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(priority, 5);
    Ok(())
}

fn find_node<'a>(
    plan: &'a serde_json::Value,
    node_type: &str,
    field: &str,
    value: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    match plan {
        serde_json::Value::Object(node) => {
            if node.get("Node Type").and_then(serde_json::Value::as_str) == Some(node_type)
                && node.get(field).and_then(serde_json::Value::as_str) == Some(value)
            {
                return Some(node);
            }
            node.values()
                .find_map(|child| find_node(child, node_type, field, value))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|item| find_node(item, node_type, field, value)),
        _ => None,
    }
}

fn plan_contains(plan: &serde_json::Value, node_type: &str, field: &str, value: &str) -> bool {
    find_node(plan, node_type, field, value).is_some()
}

#[sqlx::test(migrations = "../api/migrations")]
async fn wave_priority_reads_active_users_from_the_covering_index(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
         SELECT (n % 2000)::text, (n % 5000)::text, 'full_play', 0.3,
                now() - interval '400 days' * ((50000 - n)::float8 / 50000)
         FROM generate_series(1, 50000) AS n",
    )
    .execute(&pool)
    .await?;
    sqlx::query("VACUUM ANALYZE user_events")
        .execute(&pool)
        .await?;

    let sql = include_str!("../../../queries/recommendations/wave_priority/bump.sql");
    let plan: serde_json::Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {sql}"))
        .bind(1_i64)
        .bind(0_i64)
        .fetch_one(&pool)
        .await?;

    let scan = find_node(
        &plan,
        "Index Only Scan",
        "Index Name",
        "user_events_created_type_cover_idx",
    )
    .unwrap_or_else(|| panic!("active users must come from the covering index: {plan}"));
    let condition = scan
        .get("Index Cond")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert!(
        condition.contains("created_at"),
        "the covering index must be walked by the recency window, not end to end: {plan}"
    );
    assert!(
        !plan_contains(&plan, "Seq Scan", "Relation Name", "user_events"),
        "the bump must not read the whole event log: {plan}"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn sharding_splits_the_active_users_without_losing_any(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist_id = Uuid::from_u128(1);
    sqlx::query("INSERT INTO artists (id) VALUES ($1)")
        .bind(artist_id)
        .execute(&pool)
        .await?;
    insert_artist_track(&pool, artist_id, Uuid::from_u128(10), "seed-track", "ok", 5).await?;
    for listener in 1..=40_u128 {
        sqlx::query("INSERT INTO user_events (id, sc_user_id) VALUES ($1, $2)")
            .bind(Uuid::from_u128(1000 + listener))
            .bind(listener.to_string())
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES ($1, 'seed-track')",
        )
        .bind(listener.to_string())
        .execute(&pool)
        .await?;
    }

    let shards = 4_i64;
    let mut covered = 0_i64;
    for shard in 0..shards {
        let counted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (
                 SELECT DISTINCT regexp_replace(sc_user_id, '^soundcloud:users:', '') AS user_id
                 FROM user_events
                 WHERE created_at > now() - interval '14 days'
                   AND ((hashtextextended(
                            regexp_replace(sc_user_id, '^soundcloud:users:', ''), 0
                        ) % $1) + $1) % $1 = $2
             ) AS shard",
        )
        .bind(shards)
        .bind(shard)
        .fetch_one(&pool)
        .await?;
        assert!(counted > 0, "shard {shard} must carry work");
        covered += counted;
    }

    assert_eq!(
        covered, 40,
        "every active user belongs to exactly one shard"
    );
    Ok(())
}
