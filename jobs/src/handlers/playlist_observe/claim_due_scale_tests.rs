use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::PgPool;

use super::repository::PlaylistObserveRepository;

const PLAYLISTS: i64 = 30_000;
const OWNERS: i64 = 1_000;
const BUFFER_BUDGET: i64 = 1_500;

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
                CASE WHEN n % 4 = 0 THEN 'clean' ELSE 'unhydrated' END,
                CASE WHEN n % 4 = 0 THEN now() + interval '5 minutes'
                     ELSE now() - interval '2 days' END
         FROM generate_series(1, $1::bigint) AS n",
    )
    .bind(PLAYLISTS)
    .bind(OWNERS)
    .execute(pool)
    .await?;
    sqlx::query("UPDATE playlist_sweep_cursor SET last_owner_sc_user_id = 'u00500'")
        .execute(pool)
        .await?;
    sqlx::raw_sql("ANALYZE playlists, playlist_membership_state")
        .execute(pool)
        .await?;
    Ok(())
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
async fn the_sweep_resumes_after_its_cursor_and_wraps_to_the_first_owner(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let owners_of = |urns: Vec<String>| -> anyhow::Result<BTreeSet<i64>> {
        urns.iter()
            .map(|urn| {
                let n: i64 = urn.trim_start_matches("soundcloud:playlists:").parse()?;
                Ok(n % OWNERS)
            })
            .collect()
    };
    let first = owners_of(repository.claim_due(16, 4, 600).await?)?;
    let second = owners_of(repository.claim_due(16, 4, 600).await?)?;

    assert_eq!(first.len(), 4);
    assert!(first.iter().all(|owner| *owner > 500));
    assert!(second.is_disjoint(&first));
    assert!(second.iter().min() > first.iter().max());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_never_reads_the_table_end_to_end_when_most_rows_are_due(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let sql = include_str!("../../../queries/playlist_observe/claim_due_playlists.sql");

    let mut transaction = pool.begin().await?;
    sqlx::query("SET LOCAL plan_cache_mode = force_generic_plan")
        .execute(&mut *transaction)
        .await?;
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}"))
        .bind(2_i32)
        .bind(8_i64)
        .bind(16_i64)
        .bind(300.0_f64)
        .fetch_one(&mut *transaction)
        .await?;

    assert!(
        !reads_table_end_to_end(&plan),
        "the sweep must not read the table end to end: {plan}"
    );
    let top = &plan[0]["Plan"];
    let touched = top["Shared Hit Blocks"].as_i64().unwrap_or(i64::MAX)
        + top["Shared Read Blocks"].as_i64().unwrap_or(i64::MAX);
    assert!(
        touched <= BUFFER_BUDGET,
        "the sweep touched {touched} pages for {PLAYLISTS} rows: {plan}"
    );
    Ok(())
}
