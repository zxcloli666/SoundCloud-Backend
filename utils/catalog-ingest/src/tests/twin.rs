use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

use super::legacy;
use crate::{Observation, ScTrackFields, TrackPriority};

static CASES: AtomicUsize = AtomicUsize::new(0);

const COMPARED: [(&str, &str, &str); 6] = [
    ("tracks", "sc_track_id", "id"),
    ("users", "sc_user_id", ""),
    ("playlists", "urn", ""),
    (
        "playlist_membership_state",
        "playlist_urn",
        "next_reconcile_at,updated_at",
    ),
    ("user_profiles", "soundcloud_user_id", "synced_at"),
    (
        "lyrics_lookup_state",
        "sc_track_id",
        "track_id,wake_message_id,next_run_at,created_at,updated_at",
    ),
];

const CLOCK_COLUMNS: [(&str, &str, &str); 3] = [
    (
        "playlist_membership_state",
        "playlist_urn",
        "next_reconcile_at",
    ),
    ("playlist_membership_state", "playlist_urn", "updated_at"),
    ("user_profiles", "soundcloud_user_id", "synced_at"),
];

pub(super) enum Step<'a> {
    Track(&'a Value, TrackPriority, Observation),
    Bump(&'a str, TrackPriority),
    AbsentTracks(&'a [Value], Observation),
    AbsentPlaylists(&'a [Value], Observation),
    AbsentUsers(&'a [Value], Observation),
    Playlist(&'a Value, Observation),
    User(&'a Value, Observation),
    Profile(&'a str, &'a Value, Observation),
    Sql(&'a str),
}

pub(super) struct Twin {
    pub legacy: PgPool,
    pub split: PgPool,
    names: [String; 2],
    stamps: Vec<DateTime<Utc>>,
    pub steps: usize,
}

fn server() -> PgConnectOptions {
    env::var("DATABASE_URL")
        .expect("DATABASE_URL names the migrated build database")
        .parse()
        .expect("DATABASE_URL")
}

async fn execute(connection: &mut PgConnection, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(connection)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

impl Twin {
    pub(super) async fn new(seed: &str) -> Twin {
        let template = server()
            .get_database()
            .expect("a database in DATABASE_URL")
            .to_owned();
        let case = format!(
            "replay_{}_{}",
            std::process::id(),
            CASES.fetch_add(1, Ordering::Relaxed)
        );
        let names = [format!("{case}_legacy"), format!("{case}_split")];
        let mut admin = PgConnection::connect_with(&server().database("postgres"))
            .await
            .expect("connect to the server");
        for name in &names {
            execute(
                &mut admin,
                &format!("CREATE DATABASE {name} TEMPLATE {template}"),
            )
            .await;
        }
        admin.close().await.unwrap();
        let legacy = pool(&names[0]).await;
        let split = pool(&names[1]).await;
        sqlx::raw_sql(super::legacy::OBSERVATION_GUARD)
            .execute(&legacy)
            .await
            .unwrap();
        for db in [&legacy, &split] {
            let mut connection = db.acquire().await.unwrap();
            execute(&mut connection, "SET session_replication_role = replica").await;
            execute(&mut connection, seed).await;
            execute(
                &mut connection,
                "RESET session_replication_role;
                 CREATE SCHEMA replay;
                 CREATE TABLE replay.seed_tracks AS SELECT sc_track_id, id FROM public.tracks",
            )
            .await;
        }
        sqlx::raw_sql("SELECT count(*) FROM public.scd_configure('core', 'core', 'split')")
            .execute(&split)
            .await
            .unwrap();
        Twin {
            legacy,
            split,
            names,
            stamps: Vec::new(),
            steps: 0,
        }
    }

    pub(super) async fn step(&mut self, step: Step<'_>) -> Value {
        let mut legacy = self.legacy.begin().await.unwrap();
        let at: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
            .fetch_one(&mut *legacy)
            .await
            .unwrap();
        let before = on_legacy(&mut legacy, &step).await;
        settle(legacy, &before).await;
        let mut split = self.split.begin().await.unwrap();
        let after = on_split(&mut split, &step, at).await;
        settle(split, &after).await;
        assert_eq!(before, after, "step {} gave different results", self.steps);
        self.stamps.push(at);
        self.steps += 1;
        after
    }

    pub(super) async fn assert_same(&self) -> Vec<String> {
        let mut report = Vec::new();
        for (table, key, masked) in COMPARED {
            let legacy = buckets(&self.legacy, table, key, masked).await;
            let split = buckets(&self.split, table, key, masked).await;
            let rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM public.{table}"))
                .fetch_one(&self.legacy)
                .await
                .unwrap();
            let differing: Vec<i32> = legacy
                .iter()
                .zip(&split)
                .filter(|(l, s)| l != s)
                .map(|(l, _)| l.0)
                .collect();
            assert!(
                legacy.len() == split.len() && differing.is_empty(),
                "{table}: buckets {differing:?} differ, legacy {} split {}",
                legacy.len(),
                split.len()
            );
            report.push(format!(
                "{table}: {rows} rows, {} buckets equal",
                legacy.len()
            ));
        }
        report.push(self.assert_ids().await);
        report.push(self.assert_clock_columns().await);
        report
    }

    async fn assert_ids(&self) -> String {
        let reid = "SELECT count(*) FROM public.tracks t JOIN replay.seed_tracks s USING (sc_track_id) WHERE t.id <> s.id";
        let not_v5 = "SELECT count(*) FROM public.tracks t LEFT JOIN replay.seed_tracks s USING (sc_track_id)
                      WHERE s.id IS NULL AND t.id <> public.scd_uuid('tracks', t.urn)";
        let seeded: i64 = sqlx::query_scalar("SELECT count(*) FROM replay.seed_tracks")
            .fetch_one(&self.split)
            .await
            .unwrap();
        let fresh: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.tracks t LEFT JOIN replay.seed_tracks s USING (sc_track_id) WHERE s.id IS NULL",
        )
        .fetch_one(&self.split)
        .await
        .unwrap();
        for db in [&self.legacy, &self.split] {
            let moved: i64 = sqlx::query_scalar(reid).fetch_one(db).await.unwrap();
            assert_eq!(moved, 0, "a seeded track changed its id");
        }
        let stray: i64 = sqlx::query_scalar(not_v5)
            .fetch_one(&self.split)
            .await
            .unwrap();
        assert_eq!(stray, 0, "a new track on the split path has no v5 id");
        format!("ids: {seeded} seeded tracks kept their id, {fresh} new tracks have v5 ids")
    }

    async fn assert_clock_columns(&self) -> String {
        let mut checked = 0;
        for (table, key, column) in CLOCK_COLUMNS {
            let sql = format!(
                "SELECT {key}, {column} FROM public.{table} WHERE {column} IS NOT NULL ORDER BY {key}"
            );
            let legacy: Vec<(String, DateTime<Utc>)> =
                sqlx::query_as(&sql).fetch_all(&self.legacy).await.unwrap();
            let split: Vec<(String, DateTime<Utc>)> =
                sqlx::query_as(&sql).fetch_all(&self.split).await.unwrap();
            assert_eq!(legacy.len(), split.len(), "{table}.{column} rows");
            for ((key, then), (split_key, now)) in legacy.iter().zip(&split) {
                assert_eq!(key, split_key);
                let seeded = (*then - *now).num_milliseconds() == 0;
                let stamped =
                    self.stamps.contains(now) && (*then - *now).num_milliseconds().abs() < 1000;
                assert!(
                    seeded || stamped,
                    "{table}.{column} for {key}: legacy {then}, split {now}"
                );
                checked += 1;
            }
        }
        format!("clock columns: {checked} values match the step stamps")
    }

    pub(super) async fn finish(self) {
        self.legacy.close().await;
        self.split.close().await;
        let mut admin = PgConnection::connect_with(&server().database("postgres"))
            .await
            .unwrap();
        for name in &self.names {
            execute(
                &mut admin,
                &format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
            )
            .await;
        }
        admin.close().await.unwrap();
    }
}

async fn pool(name: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(server().database(name))
        .await
        .unwrap_or_else(|e| panic!("pool {name}: {e}"))
}

async fn settle(transaction: sqlx::Transaction<'_, sqlx::Postgres>, outcome: &Value) {
    if outcome.get("error").is_some() {
        transaction.rollback().await.unwrap();
    } else {
        transaction.commit().await.unwrap();
    }
}

async fn buckets(db: &PgPool, table: &str, key: &str, masked: &str) -> Vec<(i32, String)> {
    let row = format!("(to_jsonb(t) - '{{{masked}}}'::text[])");
    let sql = format!(
        "SELECT (abs(hashtext(t.{key})) % 16)::int4 AS bucket,
                md5(string_agg({row}::text, E'\\n' ORDER BY {row}::text))
         FROM public.{table} t GROUP BY 1 ORDER BY 1"
    );
    let mut connection = db.acquire().await.unwrap();
    execute(&mut connection, "SET timezone = 'UTC'").await;
    sqlx::query_as(&sql)
        .fetch_all(&mut *connection)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn outcome<T>(result: Result<T, sqlx::Error>, shape: impl FnOnce(T) -> Value) -> Value {
    match result {
        Ok(value) => shape(value),
        Err(error) => json!({
            "error": error.as_database_error().and_then(|e| e.code()).map(|c| c.into_owned()),
            "protocol": matches!(error, sqlx::Error::Protocol(_)),
        }),
    }
}

async fn stored_id(connection: &mut PgConnection, sc_track_id: &str) -> Option<uuid::Uuid> {
    sqlx::query_scalar("SELECT id FROM public.tracks WHERE sc_track_id = $1")
        .bind(sc_track_id)
        .fetch_optional(connection)
        .await
        .unwrap()
}

async fn on_legacy(connection: &mut PgConnection, step: &Step<'_>) -> Value {
    match step {
        Step::Track(payload, priority, observation) => {
            let fields = ScTrackFields::from_sc(payload).expect("a track payload");
            let result =
                legacy::upsert_track(connection, &fields, *priority, *priority, *observation).await;
            track_outcome(connection, &fields, result).await
        }
        Step::Bump(sc_track_id, priority) => outcome(
            legacy::bump_track_priority(connection, sc_track_id, *priority).await,
            |()| Value::Null,
        ),
        Step::AbsentTracks(payloads, observation) => outcome(
            legacy::insert_absent_tracks(connection, payloads, *observation).await,
            |inserted| json!(format!("{inserted:?}")),
        ),
        Step::AbsentPlaylists(payloads, observation) => outcome(
            legacy::insert_absent_playlists(connection, payloads, *observation).await,
            |inserted| json!(format!("{inserted:?}")),
        ),
        Step::AbsentUsers(payloads, observation) => outcome(
            legacy::insert_absent_users(connection, payloads, *observation).await,
            |inserted| json!(format!("{inserted:?}")),
        ),
        Step::Playlist(payload, observation) => outcome(
            legacy::upsert_playlist(connection, payload, *observation).await,
            Value::from,
        ),
        Step::User(payload, observation) => outcome(
            legacy::upsert_user(connection, payload, *observation).await,
            Value::from,
        ),
        Step::Profile(user_id, profile, observation) => outcome(
            legacy::upsert_profile(connection, user_id, profile, *observation).await,
            |()| Value::Null,
        ),
        Step::Sql(sql) => {
            execute(connection, sql).await;
            Value::Null
        }
    }
}

async fn on_split(connection: &mut PgConnection, step: &Step<'_>, at: DateTime<Utc>) -> Value {
    match step {
        Step::Track(payload, priority, observation) => {
            let fields = ScTrackFields::from_sc(payload).expect("a track payload");
            let result = crate::upsert::upsert_track_at(
                connection,
                &fields,
                *priority,
                *priority,
                *observation,
                at,
            )
            .await;
            track_outcome(connection, &fields, result).await
        }
        Step::Bump(sc_track_id, priority) => outcome(
            crate::upsert::bump_track_priority_at(connection, sc_track_id, *priority, at).await,
            |()| Value::Null,
        ),
        Step::AbsentTracks(payloads, observation) => outcome(
            crate::absent::insert_absent_tracks_at(connection, payloads, *observation, at).await,
            |inserted| json!(format!("{inserted:?}")),
        ),
        Step::AbsentPlaylists(payloads, observation) => outcome(
            crate::absent::insert_absent_playlists_at(connection, payloads, *observation, at).await,
            |inserted| json!(format!("{inserted:?}")),
        ),
        Step::AbsentUsers(payloads, observation) => outcome(
            crate::absent::insert_absent_users_at(connection, payloads, *observation, at).await,
            |inserted| json!(format!("{inserted:?}")),
        ),
        Step::Playlist(payload, observation) => outcome(
            crate::playlist::upsert_playlist_at(connection, payload, *observation, at).await,
            Value::from,
        ),
        Step::User(payload, observation) => outcome(
            crate::user::upsert_user_at(connection, payload, *observation, at).await,
            Value::from,
        ),
        Step::Profile(user_id, profile, observation) => outcome(
            crate::user::upsert_profile_at(connection, user_id, profile, *observation, at).await,
            |()| Value::Null,
        ),
        Step::Sql(sql) => {
            execute(connection, sql).await;
            Value::Null
        }
    }
}

async fn track_outcome(
    connection: &mut PgConnection,
    fields: &ScTrackFields,
    result: Result<crate::IngestResult, sqlx::Error>,
) -> Value {
    match result {
        Ok(ingested) => {
            assert_eq!(
                stored_id(connection, &fields.sc_track_id).await,
                Some(ingested.id),
                "the reported id is the stored id"
            );
            json!({"was_new": ingested.was_new, "applied": ingested.metadata_applied})
        }
        Err(error) => outcome(Err::<(), _>(error), |()| Value::Null),
    }
}
