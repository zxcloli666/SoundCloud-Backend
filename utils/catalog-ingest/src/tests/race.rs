use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

use crate::{Observation, ScTrackFields, TrackPriority, upsert_from_sc};

const RACERS: i64 = 6;

fn server() -> PgConnectOptions {
    std::env::var("DATABASE_URL")
        .expect("DATABASE_URL names the migrated build database")
        .parse()
        .expect("DATABASE_URL")
}

async fn wait_for_lock_waiters(db: &PgPool, waiting: i64) {
    for _ in 0..500 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if blocked >= waiting {
            return;
        }
        sqlx::query("SELECT pg_sleep(0.01)")
            .execute(db)
            .await
            .unwrap();
    }
    panic!("the racers never queued behind the gate");
}

#[tokio::test]
async fn first_upserts_of_one_new_track_racing_on_its_unique_keys_all_land_on_one_row() {
    let template = server()
        .get_database()
        .expect("a database in DATABASE_URL")
        .to_owned();
    let name = format!("race_{}", std::process::id());
    let mut admin = PgConnection::connect_with(&server().database("postgres"))
        .await
        .unwrap();
    sqlx::raw_sql(&format!("CREATE DATABASE {name} TEMPLATE {template}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let db = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(server().database(&name))
        .await
        .unwrap();

    let observation = Observation::begin(&db).await.unwrap();
    let fields = ScTrackFields::from_sc(&json!({
        "kind": "track",
        "urn": "soundcloud:tracks:7700001",
        "title": "Raced",
        "full_duration": 120000,
        "sharing": "public"
    }))
    .unwrap();
    let mut gate = db.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ($1, $2, 'Gate', 'gate', 1000)",
    )
    .bind(&fields.sc_track_id)
    .bind(&fields.urn)
    .execute(&mut *gate)
    .await
    .unwrap();

    let upsert = || {
        upsert_from_sc(
            &db,
            &fields,
            TrackPriority::Discovery,
            TrackPriority::Discovery,
            observation,
        )
    };
    let released = async {
        wait_for_lock_waiters(&db, RACERS).await;
        gate.rollback().await
    };
    let (a, b, c, d, e, f, released) = tokio::join!(
        upsert(),
        upsert(),
        upsert(),
        upsert(),
        upsert(),
        upsert(),
        released
    );
    released.unwrap();
    let errors: Vec<String> = [a, b, c, d, e, f]
        .into_iter()
        .filter_map(|result| result.err().map(|error| error.to_string()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let stored: (i64, bool, String) = sqlx::query_as(
        "SELECT count(*), bool_and(id = public.scd_uuid('tracks', urn)), min(title)
         FROM tracks WHERE sc_track_id = $1",
    )
    .bind(&fields.sc_track_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(stored, (1, true, "Raced".to_owned()));

    db.close().await;
    sqlx::raw_sql(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&mut admin)
        .await
        .unwrap();
    admin.close().await.unwrap();
}
