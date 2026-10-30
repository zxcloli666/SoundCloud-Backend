use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::postgres::PgDatabaseError;
use sqlx::{Connection, PgConnection, PgPool};
use write_through as wt;
use wt::{Kind, Raced};

use crate::db;
use crate::harness::Pair;

pub const HOLD: i64 = 7424202;
pub const ID: &str = "00000000-0000-0000-0000-0000000000c1";
pub const OTHER_ID: &str = "00000000-0000-0000-0000-0000000000c2";
pub const URN: &str = "urn:race:1";

#[derive(Debug, Serialize)]
struct Race {
    id: &'static str,
    urn: &'static str,
    note: &'static str,
}

impl Kind for Race {
    const NAME: &'static str = "test.race";
    type Output = Value;
}

const KIND: Race = Race {
    id: ID,
    urn: URN,
    note: "kind",
};

pub struct Holder {
    pub id: &'static str,
    pub applied: bool,
}

fn error(result: &Result<Value, sqlx::Error>) -> (String, Option<String>) {
    let error = result.as_ref().expect_err("expected a unique violation");
    let pg = error
        .as_database_error()
        .and_then(|e| e.try_downcast_ref::<PgDatabaseError>())
        .unwrap_or_else(|| panic!("not a database error: {error}"));
    (pg.code().to_owned(), pg.hint().map(str::to_owned))
}

pub async fn insert_holder(db: &str, holder: &Holder) {
    let mut conn = db::connect(db).await;
    let origin = format!("{db}_apply");
    if holder.applied {
        sqlx::query("SELECT pg_replication_origin_create($1)")
            .bind(&origin)
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::query("SELECT pg_replication_origin_session_setup($1)")
            .bind(&origin)
            .execute(&mut conn)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO wt_test.race (id, urn, note) VALUES ($1::uuid, $2, 'holder')")
        .bind(holder.id)
        .bind(URN)
        .execute(&mut conn)
        .await
        .unwrap();
    if holder.applied {
        sqlx::query("SELECT pg_replication_origin_session_reset()")
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::query("SELECT pg_replication_origin_drop($1)")
            .bind(&origin)
            .execute(&mut conn)
            .await
            .unwrap();
    }
    conn.close().await.unwrap();
}

async fn wait_for_hold(ctl: &mut PgConnection) {
    for _ in 0..400 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND objid = $1 AND NOT granted
                AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))",
        )
        .bind(HOLD)
        .fetch_one(&mut *ctl)
        .await
        .unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the contender never reached the hold index");
}

pub async fn race<T: Send + 'static>(
    db: &str,
    holder: Holder,
    contender: impl Future<Output = T> + Send + 'static,
) -> T {
    let mut ctl = db::connect(db).await;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(HOLD)
        .execute(&mut ctl)
        .await
        .unwrap();
    let task = tokio::spawn(contender);
    wait_for_hold(&mut ctl).await;
    insert_holder(db, &holder).await;
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(HOLD)
        .execute(&mut ctl)
        .await
        .unwrap();
    let out = task.await.unwrap();
    ctl.close().await.unwrap();
    out
}

async fn held_call(pool: PgPool, attempts: Arc<AtomicUsize>) -> Result<Value, sqlx::Error> {
    wt::retry(|| async {
        attempts.fetch_add(1, Ordering::SeqCst);
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL wt.hold = 'on'")
            .execute(&mut *tx)
            .await?;
        let output = wt::call(&mut tx, &KIND).await?;
        tx.commit().await.map(|()| output)
    })
    .await
}

pub async fn rows(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'note', note) ORDER BY id), '[]') FROM wt_test.race")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn outbox(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM public.cmd_outbox")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_concurrent_apply_insert_of_the_same_key_is_retried_once_on_api_and_core() {
    let pair = Pair::new("").await;
    for (db, pool) in [(&pair.api_db, &pair.api), (&pair.core_db, &pair.core)] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let holder = Holder {
            id: ID,
            applied: true,
        };
        let result = race(db, holder, held_call(pool.clone(), attempts.clone())).await;
        assert_eq!(result.unwrap(), json!({"was_new": false}), "on {db}");
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "on {db}");
        assert_eq!(
            rows(pool).await,
            json!([{"id": ID, "note": "kind"}]),
            "on {db}"
        );
    }
    assert_eq!(outbox(&pair.api).await, 1);
    assert_eq!(outbox(&pair.core).await, 0);
    pair.finish().await;
}

#[tokio::test]
async fn without_the_retry_the_race_is_the_lab_3b_unique_violation() {
    let pair = Pair::new("").await;
    let pool = pair.api.clone();
    let contender = async move {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL wt.hold = 'on'")
            .execute(&mut *tx)
            .await
            .unwrap();
        wt::call(&mut tx, &KIND).await
    };
    let result = race(
        &pair.api_db,
        Holder {
            id: ID,
            applied: true,
        },
        contender,
    )
    .await;
    assert_eq!(
        error(&result),
        ("23505".to_owned(), Some(wt::RACE_HINT.to_owned()))
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_row_that_existed_before_is_a_real_violation_and_is_not_retried() {
    let pair = Pair::new("").await;
    insert_holder(
        &pair.api_db,
        &Holder {
            id: OTHER_ID,
            applied: true,
        },
    )
    .await;
    let attempts = AtomicUsize::new(0);
    let result = wt::retry(|| async {
        attempts.fetch_add(1, Ordering::SeqCst);
        let mut tx = pair.api.begin().await?;
        let output = wt::call(&mut tx, &KIND).await?;
        tx.commit().await.map(|()| output)
    })
    .await;
    assert_eq!(error(&result), ("23505".to_owned(), None));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(outbox(&pair.api).await, 0);
    pair.finish().await;
}

#[tokio::test]
async fn a_concurrent_local_insert_is_not_retried() {
    let pair = Pair::new("").await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let result = race(
        &pair.api_db,
        Holder {
            id: ID,
            applied: false,
        },
        held_call(pair.api.clone(), attempts.clone()),
    )
    .await;
    assert_eq!(error(&result), ("23505".to_owned(), None));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    pair.finish().await;
}

#[tokio::test]
async fn a_concurrent_apply_insert_under_another_id_is_retried_once_then_surfaces() {
    let pair = Pair::new("").await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let result = race(
        &pair.api_db,
        Holder {
            id: OTHER_ID,
            applied: true,
        },
        held_call(pair.api.clone(), attempts.clone()),
    )
    .await;
    assert_eq!(error(&result), ("23505".to_owned(), None));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        rows(&pair.api).await,
        json!([{"id": OTHER_ID, "note": "holder"}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_unique_violation_outside_scd_wt_is_not_retried() {
    let pair = Pair::new("").await;
    insert_holder(
        &pair.api_db,
        &Holder {
            id: ID,
            applied: true,
        },
    )
    .await;
    let attempts = AtomicUsize::new(0);
    let result = wt::retry(|| async {
        attempts.fetch_add(1, Ordering::SeqCst);
        let mut tx = pair.api.begin().await?;
        sqlx::query("INSERT INTO wt_test.race (id, urn) VALUES ($1::uuid, 'urn:race:2')")
            .bind(ID)
            .execute(&mut *tx)
            .await?;
        tx.commit().await.map(|()| Value::Null)
    })
    .await;
    assert_eq!(error(&result), ("23505".to_owned(), None));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    pair.finish().await;
}

#[tokio::test]
async fn tokio_postgres_client_retries_the_race_once() {
    let pair = Pair::new("").await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let db = pair.api_db.clone();
    let contender = async move {
        wt::retry(|| async {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut client = db::tokio_client(&db).await;
            let tx = client.transaction().await?;
            tx.batch_execute("SET LOCAL wt.hold = 'on'").await?;
            let output = wt::tokio_pg::call(&tx, &KIND).await?;
            tx.commit().await.map(|()| output)
        })
        .await
        .map_err(|e| e.to_string())
    };
    let result = race(
        &pair.api_db,
        Holder {
            id: ID,
            applied: true,
        },
        contender,
    )
    .await;
    assert_eq!(result.unwrap(), json!({"was_new": false}));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(outbox(&pair.api).await, 1);
    pair.finish().await;
}

#[tokio::test]
async fn tokio_postgres_client_does_not_retry_a_real_violation() {
    let pair = Pair::new("").await;
    insert_holder(
        &pair.api_db,
        &Holder {
            id: OTHER_ID,
            applied: true,
        },
    )
    .await;
    let attempts = AtomicUsize::new(0);
    let error = wt::retry(|| async {
        attempts.fetch_add(1, Ordering::SeqCst);
        let mut client = db::tokio_client(&pair.api_db).await;
        let tx = client.transaction().await?;
        let output = wt::tokio_pg::call(&tx, &KIND).await?;
        tx.commit().await.map(|()| output)
    })
    .await
    .unwrap_err();
    let db_error = error.as_db_error().expect("database error");
    assert_eq!((db_error.code().code(), db_error.hint()), ("23505", None));
    assert!(!error.raced());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    pair.finish().await;
}
