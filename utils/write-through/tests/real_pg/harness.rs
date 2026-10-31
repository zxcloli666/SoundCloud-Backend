use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Connection, PgPool};
use tokio::sync::{Semaphore, SemaphorePermit};
use write_through as wt;
use wt::Kind;

use crate::db::{self, hashes};

static SLOTS: Semaphore = Semaphore::const_new(2);
static CASES: AtomicUsize = AtomicUsize::new(0);

pub struct Pair {
    pub api: PgPool,
    pub core: PgPool,
    pub api_db: String,
    pub core_db: String,
    _slot: SemaphorePermit<'static>,
}

pub struct Run {
    pub api: Value,
    pub core: Value,
    pub diverged: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct Raw<'a, K>(&'a K);

impl<K: Kind> Kind for Raw<'_, K> {
    const NAME: &'static str = K::NAME;
    type Output = Value;
}

struct Command {
    id: String,
    kind: String,
    args: Value,
    touched: Value,
    keys: i64,
    pending: i64,
}

#[derive(Debug, PartialEq)]
struct Verdict {
    status: Option<String>,
    reasserted: i64,
    purged: i64,
    dead: i64,
}

impl Pair {
    pub async fn new(fixture: &str) -> Pair {
        let slot = SLOTS.acquire().await.unwrap();
        let template = db::template().await;
        let case = format!(
            "wt_{}_{}",
            std::process::id(),
            CASES.fetch_add(1, Ordering::Relaxed)
        );
        let core_db = format!("{case}_core");
        let api_db = format!("{case}_api");
        let mut admin = db::admin().await;
        db::execute(
            &mut admin,
            &format!("CREATE DATABASE {core_db} TEMPLATE {template}"),
        )
        .await;
        let mut core = db::connect(&core_db).await;
        db::execute(
            &mut core,
            "SELECT count(*) FROM public.scd_configure('core', 'core', 'split')",
        )
        .await;
        db::execute(&mut core, fixture).await;
        core.close().await.unwrap();
        db::execute(
            &mut admin,
            &format!("CREATE DATABASE {api_db} TEMPLATE {core_db}"),
        )
        .await;
        let mut api = db::connect(&api_db).await;
        db::execute(
            &mut api,
            "SELECT count(*) FROM public.scd_configure('main', 'api', 'split')",
        )
        .await;
        api.close().await.unwrap();
        admin.close().await.unwrap();
        Pair {
            api: db::pool(&api_db).await,
            core: db::pool(&core_db).await,
            api_db,
            core_db,
            _slot: slot,
        }
    }

    pub async fn finish(self) {
        self.api.close().await;
        self.core.close().await;
        let mut admin = db::admin().await;
        for name in [&self.api_db, &self.core_db] {
            db::execute(
                &mut admin,
                &format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
            )
            .await;
        }
        admin.close().await.unwrap();
    }

    pub async fn run<K: Kind>(&self, args: &K, derived: &[&str]) -> Run {
        let raw = Raw(args);
        let optimistic: bool =
            sqlx::query_scalar("SELECT optimistic FROM public.scd_kinds() WHERE kind = $1")
                .bind(K::NAME)
                .fetch_one(&self.core)
                .await
                .unwrap_or_else(|e| panic!("{} is not a registered kind: {e}", K::NAME));
        let before = hashes(&self.api).await;
        let api = wt::retry(|| async {
            let mut tx = self.api.begin().await?;
            let output = wt::call(&mut tx, &raw).await?;
            tx.commit().await.map(|()| output)
        })
        .await
        .unwrap_or_else(|e| panic!("{} on the api node: {e}", K::NAME));
        let command = Command::undelivered(&self.api).await;
        let mut local = self.core.begin().await.unwrap();
        let core = wt::call(&mut local, &raw)
            .await
            .unwrap_or_else(|e| panic!("{} on core: {e}", K::NAME));
        let core_local = hashes(&mut *local).await;
        local.rollback().await.unwrap();
        let verdict = command.dispatch(&self.core).await;
        command
            .ack(&self.api, verdict.status.as_deref().unwrap_or("dead"))
            .await;
        let api_after = hashes(&self.api).await;
        let core_after = hashes(&self.core).await;

        let mut diverged = Vec::new();
        if command.kind != K::NAME || command.args != serde_json::to_value(args).unwrap() {
            diverged.push(format!("outbox holds {} {}", command.kind, command.args));
        }
        if optimistic {
            if api != core {
                diverged.push(format!("output: api {api}, core {core}"));
            }
            if command.pending != command.keys {
                diverged.push(format!(
                    "pending: {} rows for {} touched keys",
                    command.pending, command.keys
                ));
            }
            for t in differ(&api_after, &core_after, derived) {
                diverged.push(format!("api and core differ: {t}"));
            }
        } else {
            if !api.is_null() || command.touched != json!([]) {
                diverged.push(format!(
                    "the api ran a non-optimistic kind: output {api}, touched {}",
                    command.touched
                ));
            }
            for t in differ(&before, &api_after, &[]) {
                diverged.push(format!("the api changed {t} for a non-optimistic kind"));
            }
        }
        for t in differ(&core_local, &core_after, derived) {
            diverged.push(format!("core local run and dispatch differ: {t}"));
        }
        let clean = Verdict {
            status: Some("ok".to_owned()),
            reasserted: 0,
            purged: 0,
            dead: 0,
        };
        if verdict != clean {
            diverged.push(format!("dispatch on core: {verdict:?}"));
        }
        Run {
            api,
            core,
            diverged,
        }
    }
}

impl Run {
    pub fn assert_same(self) -> Value {
        assert!(
            self.diverged.is_empty(),
            "api and core diverged: {:#?}",
            self.diverged
        );
        self.api
    }
}

impl Command {
    async fn undelivered(api: &PgPool) -> Command {
        let mut rows = sqlx::query_as::<_, (String, String, Value, Value, i64, i64)>(
            "SELECT o.id::text, o.kind, o.args, o.touched,
                    (SELECT count(DISTINCT (e ->> 'tbl', e -> 'pk')) FROM jsonb_array_elements(o.touched) e),
                    (SELECT count(*) FROM public.cmd_pending p WHERE p.id = o.id)
             FROM public.cmd_outbox o WHERE o.done_at IS NULL",
        )
        .fetch_all(api)
        .await
        .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "the api must hold exactly one undelivered command"
        );
        let (id, kind, args, touched, keys, pending) = rows.remove(0);
        Command {
            id,
            kind,
            args,
            touched,
            keys,
            pending,
        }
    }

    async fn dispatch(&self, core: &PgPool) -> Verdict {
        let mut tx = core.begin().await.unwrap();
        sqlx::query("SET LOCAL session_replication_role = replica")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.cmd_outbox (node, id, kind, args, touched) VALUES ('main', $1::uuid, $2, $3, $4)")
            .bind(&self.id)
            .bind(&self.kind)
            .bind(&self.args)
            .bind(&self.touched)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("dispatch of {} on core: {e}", self.kind));
        tx.commit().await.unwrap();
        let (status, reasserted, purged, dead) = sqlx::query_as::<_, (Option<String>, i64, i64, i64)>(
            "SELECT (SELECT status FROM public.cmd_done WHERE node = 'main' AND id = $1::uuid),
                    (SELECT count(*) FROM public.cmd_reassert WHERE node = 'main' AND id = $1::uuid),
                    (SELECT count(*) FROM public.cmd_purge WHERE node = 'main' AND id = $1::uuid),
                    (SELECT count(*) FROM public.cmd_dead WHERE node = 'main' AND id = $1::uuid)",
        )
        .bind(&self.id)
        .fetch_one(core)
        .await
        .unwrap();
        Verdict {
            status,
            reasserted,
            purged,
            dead,
        }
    }

    async fn ack(&self, api: &PgPool, status: &str) {
        let mut tx = api.begin().await.unwrap();
        sqlx::query("SET LOCAL session_replication_role = replica")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.cmd_done (node, id, status) VALUES ('main', $1::uuid, $2)")
            .bind(&self.id)
            .bind(status)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
}

fn differ<'a>(
    a: &'a BTreeMap<String, String>,
    b: &'a BTreeMap<String, String>,
    skip: &[&str],
) -> Vec<&'a str> {
    a.iter()
        .filter(|(t, h)| b.get(*t) != Some(*h) && !skip.contains(&t.as_str()))
        .map(|(t, _)| t.as_str())
        .collect()
}
