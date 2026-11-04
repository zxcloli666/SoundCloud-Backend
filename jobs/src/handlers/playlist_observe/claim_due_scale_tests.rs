use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::PgPool;

use super::repository::PlaylistObserveRepository;

const PLAYLISTS: i64 = 30_000;
const OWNERS: i64 = 1_000;
const DUE_EVERY: i64 = 1_000;
const BUFFER_BUDGET: i64 = 100;

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id)
         SELECT n::text, 'soundcloud:playlists:' || n, 'Mix', 'mix',
                'u' || lpad((n % $2)::text, 5, '0')
         FROM generate_series(1, $1::bigint) AS n",
    )
    .bind(PLAYLISTS)
    .bind(OWNERS)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state
             (playlist_urn, owner_sc_user_id, sync_status, next_reconcile_at)
         SELECT 'soundcloud:playlists:' || n, 'u' || lpad((n % $2)::text, 5, '0'),
                CASE WHEN n % 4 = 0 THEN 'clean' ELSE 'shadow_ready' END,
                CASE
                    WHEN n % $3 = 1 THEN now() - interval '1 minute'
                    ELSE now() + interval '5 hours'
                END
         FROM generate_series(1, $1::bigint) AS n",
    )
    .bind(PLAYLISTS)
    .bind(OWNERS)
    .bind(DUE_EVERY)
    .execute(pool)
    .await?;
    sqlx::raw_sql("ANALYZE playlists, playlist_membership_state")
        .execute(pool)
        .await?;
    Ok(())
}

fn due_urns() -> BTreeSet<String> {
    (1..=PLAYLISTS)
        .filter(|n| n % DUE_EVERY == 1)
        .map(|n| format!("soundcloud:playlists:{n}"))
        .collect()
}

fn named_node<'a>(plan: &'a Value, name: &str) -> Option<&'a Value> {
    match plan {
        Value::Object(map) => {
            if map.get("Subplan Name") == Some(&Value::from(name)) {
                return Some(plan);
            }
            map.values().find_map(|child| named_node(child, name))
        }
        Value::Array(items) => items.iter().find_map(|item| named_node(item, name)),
        _ => None,
    }
}

fn reads_table_end_to_end(plan: &Value) -> bool {
    match plan {
        Value::Object(map) => {
            (map.get("Node Type") == Some(&Value::from("Seq Scan"))
                && map.get("Relation Name") == Some(&Value::from("playlist_membership_state")))
                || map.values().any(reads_table_end_to_end)
        }
        Value::Array(items) => items.iter().any(reads_table_end_to_end),
        _ => false,
    }
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_serves_every_due_playlist_among_many_waiting_ones(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let mut served = BTreeSet::new();
    for _ in 0..20 {
        let claimed = repository.claim_due(16, 4, 600).await?;
        if claimed.is_empty() {
            break;
        }
        served.extend(claimed);
    }

    assert_eq!(served, due_urns());
    assert!(repository.claim_due(16, 4, 600).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_reads_only_the_due_entries_of_the_index(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let sql = include_str!("../../../queries/playlist_observe/claim_due_playlists.sql");

    let mut transaction = pool.begin().await?;
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}"))
        .bind(4_i64)
        .bind(4_i64)
        .bind(16_i64)
        .bind(600.0_f64)
        .fetch_one(&mut *transaction)
        .await?;

    let text = plan.to_string();
    assert!(
        text.contains("playlist_membership_state_due_owner_idx"),
        "due owners must come from the due index: {plan}"
    );
    assert!(
        !reads_table_end_to_end(&plan),
        "the sweep must not read the table end to end: {plan}"
    );
    let owners = named_node(&plan, "CTE due_owners")
        .ok_or_else(|| anyhow::anyhow!("no due owners step: {plan}"))?;
    let touched = owners["Shared Hit Blocks"].as_i64().unwrap_or(i64::MAX)
        + owners["Shared Read Blocks"].as_i64().unwrap_or(i64::MAX);
    assert!(
        touched <= BUFFER_BUDGET,
        "finding the due owners touched {touched} pages for {PLAYLISTS} waiting rows: {plan}"
    );
    Ok(())
}
