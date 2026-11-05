use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::PgPool;

use super::repository::PlaylistObserveRepository;

const PLAYLISTS: i64 = 30_000;
const OWNERS: i64 = 1_000;
const BUFFER_BUDGET: i64 = 1_500;

async fn seed(pool: &PgPool, due_percent: i64) -> anyhow::Result<()> {
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
                CASE WHEN n % 4 <> 0 AND n % 100 < $3 THEN now() - interval '2 days'
                     ELSE now() + interval '5 hours' END
         FROM generate_series(1, $1::bigint) AS n",
    )
    .bind(PLAYLISTS)
    .bind(OWNERS)
    .bind(due_percent)
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
    seed(&pool, 100).await?;
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

async fn pages_touched(pool: &PgPool) -> anyhow::Result<(i64, Value)> {
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
    let top = &plan[0]["Plan"];
    let touched = top["Shared Hit Blocks"].as_i64().unwrap_or(i64::MAX)
        + top["Shared Read Blocks"].as_i64().unwrap_or(i64::MAX);
    Ok((touched, plan))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_sweep_with_few_due_playlists_does_not_walk_the_waiting_ones(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1).await?;

    let (touched, plan) = pages_touched(&pool).await?;

    assert!(!reads_table_end_to_end(&plan), "{plan}");
    assert!(touched <= BUFFER_BUDGET, "touched {touched} pages: {plan}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_sweep_with_nothing_due_reads_a_few_pages(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, 0).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let (touched, plan) = pages_touched(&pool).await?;

    assert!(touched <= 100, "touched {touched} pages: {plan}");
    assert!(repository.claim_due(16, 4, 600).await?.is_empty());
    Ok(())
}

async fn seed_edges(pool: &PgPool) -> anyhow::Result<()> {
    for (owner, urn, ago) in [
        ("", "e1", 2),
        ("", "e2", 2),
        ("a", "a1", 2),
        ("a", "a2", 1),
        ("b", "b1", 2),
        ("b", "b2", 2),
    ] {
        sqlx::query(
            "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id)
             VALUES ($1, $1, 'Mix', 'mix', $2)",
        )
        .bind(urn)
        .bind(owner)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO playlist_membership_state
                 (playlist_urn, owner_sc_user_id, sync_status, next_reconcile_at)
             VALUES ($1, $2, 'pending', date_trunc('second', now()) - $3 * interval '1 hour')",
        )
        .bind(urn)
        .bind(owner)
        .bind(f64::from(ago))
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn cursor_at(pool: &PgPool, owner: Option<&str>) -> anyhow::Result<()> {
    sqlx::query("UPDATE playlist_sweep_cursor SET last_owner_sc_user_id = $1")
        .bind(owner)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_starts_at_the_empty_owner_without_a_cursor(pool: PgPool) -> anyhow::Result<()> {
    seed_edges(&pool).await?;
    cursor_at(&pool, None).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let claimed = repository.claim_due(1, 1, 600).await?;

    assert_eq!(claimed, vec!["e1".to_owned()]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_cursor_on_the_last_owner_wraps_to_the_first(pool: PgPool) -> anyhow::Result<()> {
    seed_edges(&pool).await?;
    cursor_at(&pool, Some("b")).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let claimed = repository.claim_due(1, 1, 600).await?;

    assert_eq!(claimed, vec!["e1".to_owned()]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn owners_are_served_in_order_oldest_first_with_ties_by_urn(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_edges(&pool).await?;
    cursor_at(&pool, Some("")).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);

    let first = repository.claim_due(2, 2, 600).await?;
    let second = repository.claim_due(2, 2, 600).await?;

    assert_eq!(first, vec!["a1".to_owned(), "a2".to_owned()]);
    assert_eq!(second, vec!["b1".to_owned(), "b2".to_owned()]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_never_reads_the_table_end_to_end_when_most_rows_are_due(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 100).await?;
    let (touched, plan) = pages_touched(&pool).await?;

    assert!(
        !reads_table_end_to_end(&plan),
        "the sweep must not read the table end to end: {plan}"
    );
    assert!(
        touched <= BUFFER_BUDGET,
        "the sweep touched {touched} pages for {PLAYLISTS} rows: {plan}"
    );
    Ok(())
}
