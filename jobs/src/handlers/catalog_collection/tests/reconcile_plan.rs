use serde_json::Value;
use sqlx::PgPool;

const SMALL_LISTS: i64 = 20_000;
const SEEN: i64 = 3_000;
const LISTED: i64 = 3_200;
const SNAPSHOT: &str = "00000000-0000-0000-0000-00000000000b";

struct Reconcile {
    target: &'static str,
    types: &'static str,
    sql: &'static str,
    args: String,
}

fn reconciles() -> [Reconcile; 2] {
    [
        Reconcile {
            target: "catalog_audience_ids",
            types: "bigint, smallint, timestamptz, text, text, uuid",
            sql: include_str!("../../../../queries/catalog_collection/audience_ids_reconcile.sql"),
            args: format!("0, 1, now(), 'big', 'track-favoriters', '{SNAPSHOT}'"),
        },
        Reconcile {
            target: "user_followers",
            types: "text, timestamptz, uuid, bigint",
            sql: include_str!("../../../../queries/catalog_collection/reconcile_followers.sql"),
            args: format!("'big', now(), '{SNAPSHOT}', 5000"),
        },
    ]
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(&format!(
        "CREATE TEMP TABLE list AS
         SELECT 's' || n AS subject, n AS id, 2 AS size, md5('s' || n)::uuid AS snapshot_id, 2 AS seen
         FROM generate_series(1, {SMALL_LISTS}) AS n
         UNION ALL SELECT 'big', 0, {LISTED}, '{SNAPSHOT}'::uuid, {SEEN};
         INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id)
         SELECT list.subject, collection, 'public', gen_random_uuid(), 1, list.snapshot_id
         FROM list, unnest(ARRAY['track-favoriters', 'followers']) AS collection;
         INSERT INTO catalog_collection_seen (subject_id, collection, scope, entity_key, snapshot_id)
         SELECT list.subject, collection, 'public', 'soundcloud:users:' || k, list.snapshot_id
         FROM list, unnest(ARRAY['track-favoriters', 'followers']) AS collection, generate_series(1, list.seen) AS k;
         INSERT INTO catalog_audience_ids (subject_id, relation, user_id, created_at)
         SELECT list.id, 1, k, now() - interval '1 day'
         FROM list, generate_series(1, list.size) AS k;
         INSERT INTO user_followers (user_id, target_user_urn, created_at)
         SELECT list.subject, 'soundcloud:users:' || k, now() - interval '1 day'
         FROM list, generate_series(1, list.size) AS k;
         DROP TABLE list;
         ANALYZE catalog_collection_sync, catalog_collection_seen, catalog_audience_ids, user_followers"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn nodes<'a>(plan: &'a Value, out: &mut Vec<&'a Value>) {
    match plan {
        Value::Object(map) => {
            if map.contains_key("Node Type") {
                out.push(plan);
            }
            map.values().for_each(|child| nodes(child, out));
        }
        Value::Array(items) => items.iter().for_each(|item| nodes(item, out)),
        _ => {}
    }
}

fn number(node: &Value, key: &str) -> f64 {
    node[key].as_f64().unwrap_or(f64::INFINITY)
}

async fn generic_plan(pool: &PgPool, reconcile: &Reconcile) -> anyhow::Result<Value> {
    let mut transaction = pool.begin().await?;
    sqlx::raw_sql(&format!(
        "SET LOCAL plan_cache_mode = force_generic_plan;
         PREPARE reconcile({}) AS {}",
        reconcile.types, reconcile.sql
    ))
    .execute(&mut *transaction)
    .await?;
    let plan: Value = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, FORMAT JSON) EXECUTE reconcile({})",
        reconcile.args
    ))
    .persistent(false)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::raw_sql("DEALLOCATE reconcile")
        .execute(&mut *transaction)
        .await?;
    transaction.rollback().await?;
    Ok(plan)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_reconcile_probes_the_seen_key_once_per_row_even_when_the_snapshot_looks_rare(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    for reconcile in reconciles() {
        let plan = generic_plan(&pool, &reconcile).await?;
        let target = reconcile.target;
        let mut all = Vec::new();
        nodes(&plan, &mut all);
        let seen_reads: f64 = all
            .iter()
            .filter(|node| node["Relation Name"] == "catalog_collection_seen")
            .map(|node| number(node, "Actual Rows") * number(node, "Actual Loops"))
            .sum();
        assert!(
            seen_reads <= 2.0 * LISTED as f64,
            "{target} reads {seen_reads} seen rows for {LISTED} listed ones: {plan}"
        );
        let matched: f64 = all
            .iter()
            .filter(|node| node.get("Rows Removed by Join Filter").is_some())
            .map(|node| number(node, "Rows Removed by Join Filter"))
            .sum();
        assert_eq!(
            matched, 0.0,
            "{target} matches rows by a join filter: {plan}"
        );
        let deleted = all
            .iter()
            .find(|node| node["Node Type"] == "ModifyTable")
            .map(|node| number(&node["Plans"][0], "Actual Rows"));
        assert_eq!(
            deleted,
            Some((LISTED - SEEN) as f64),
            "{target} must drop exactly the rows this snapshot did not see: {plan}"
        );
    }
    Ok(())
}
