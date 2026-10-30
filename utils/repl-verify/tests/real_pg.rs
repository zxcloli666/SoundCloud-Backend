use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;

use repl_verify::{Mode, Sweeper, Verifier, probe_once, repair_once, sweep_once};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

static TEMPLATE: OnceCell<String> = OnceCell::const_new();

fn options(db: &str) -> PgConnectOptions {
    env::var("DATABASE_URL")
        .expect("DATABASE_URL names a Postgres 17 superuser")
        .parse::<PgConnectOptions>()
        .expect("DATABASE_URL")
        .database(db)
}

async fn admin() -> PgConnection {
    PgConnection::connect_with(&options("postgres"))
        .await
        .expect("connect to the server")
}

async fn execute(conn: &mut PgConnection, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn template() -> &'static str {
    TEMPLATE
        .get_or_init(|| async {
            let name = format!("rv_tpl_{}", std::process::id());
            let mut admin = admin().await;
            execute(
                &mut admin,
                &format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
            )
            .await;
            execute(&mut admin, &format!("CREATE DATABASE {name}")).await;
            let mut db = PgConnection::connect_with(&options(&name)).await.unwrap();
            execute(&mut db, "SET client_min_messages = error").await;
            let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/migrations");
            let mut files: Vec<_> = fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
                .collect();
            files.sort();
            for file in files {
                sqlx::raw_sql(&fs::read_to_string(&file).unwrap())
                    .execute(&mut db)
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            }
            db.close().await.unwrap();
            name
        })
        .await
}

async fn node(case: &str, configure: Option<(&str, &str)>) -> PgPool {
    let name = format!("rv_{case}_{}", std::process::id());
    let mut admin = admin().await;
    execute(
        &mut admin,
        &format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
    )
    .await;
    execute(
        &mut admin,
        &format!("CREATE DATABASE {name} TEMPLATE {}", template().await),
    )
    .await;
    if let Some((node, role)) = configure {
        let mut db = PgConnection::connect_with(&options(&name)).await.unwrap();
        execute(
            &mut db,
            &format!("SELECT count(*) FROM public.scd_configure('{node}', '{role}', 'split')"),
        )
        .await;
        db.close().await.unwrap();
    }
    PgPoolOptions::new()
        .max_connections(3)
        .connect_with(options(&name))
        .await
        .unwrap()
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn legacy_node_sweeps_nothing() {
    let pool = node("legacy", None).await;
    let sweep = sweep_once(&pool, 500).await.unwrap();
    assert_eq!(
        (sweep.state.as_str(), sweep.work(), sweep.queued),
        ("legacy", 0, 0)
    );
    let probe = probe_once(&pool, 10, 500).await.unwrap();
    assert_eq!((probe.state.as_str(), probe.probes), ("idle", 0));
}

#[tokio::test]
async fn sweeper_drains_an_api_queue_without_a_manual_drain() {
    let pool = node("sweep", Some(("main", "api"))).await;
    sqlx::raw_sql(
        "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES (public.scd_uuid('disliked_tracks', 'rv', 'x'), 'rv', 'x');
         INSERT INTO public.repl_resend (tbl, pk) VALUES
             ('disliked_tracks', jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'rv', 'x'))),
             ('disliked_tracks', jsonb_build_object('id', public.scd_uuid('disliked_tracks', 'gone', 'x')))",
    )
    .execute(&pool)
    .await
    .unwrap();
    let stop = CancellationToken::new();
    let task = tokio::spawn(
        Sweeper::new(pool.clone())
            .with_pace(Duration::from_millis(50))
            .run(stop.clone()),
    );
    let mut left = 2;
    for _ in 0..100 {
        left = count(&pool, "SELECT count(*) FROM public.repl_resend").await;
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the sweeper stops on cancel")
        .unwrap()
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM public.repl_repair").await,
        0
    );
    let sweep = sweep_once(&pool, 500).await.unwrap();
    assert_eq!(
        (sweep.state.as_str(), sweep.work(), sweep.queued),
        ("ran", 0, 0)
    );
}

#[tokio::test]
async fn core_probes_and_settles_an_answer() {
    let pool = node("core", Some(("core", "core"))).await;
    let probe = probe_once(&pool, 100_000, 500).await.unwrap();
    assert_eq!(probe.state, "ran");
    assert!(probe.probes > 0 && probe.wrapped, "{probe:?}");
    sqlx::query(
        "INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
        VALUES ('star', 'artists', gen_random_uuid()::text,
                jsonb_build_object('lo', NULL, 'hi', NULL, 'attempt', 0, 'n', 1, 'keys', NULL)::text, now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    let rows = repair_once(&pool, false, 50).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].recheck() && rows[0].attempt == 0, "{rows:?}");
    let answer ="INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
        VALUES ('main', 'artists', gen_random_uuid()::text,
                jsonb_build_object('lo', NULL, 'hi', NULL, 'attempt', 1, 'n', 1,
                    'keys', jsonb_build_object('{\"id\": \"00000000-0000-0000-0000-0000000000e1\"}', 'x'))::text, now())";
    sqlx::query(answer).execute(&pool).await.unwrap();
    let rows = repair_once(&pool, true, 50).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(
        (
            row.peer.as_str(),
            row.rel.as_str(),
            row.k.as_deref(),
            row.action.as_str(),
            row.attempt
        ),
        (
            "main",
            "artists",
            Some("{\"id\": \"00000000-0000-0000-0000-0000000000e1\"}"),
            "drop",
            1
        )
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM public.repl_checksums WHERE node = 'main'"
        )
        .await,
        0
    );

    sqlx::query(answer).execute(&pool).await.unwrap();
    let stop = CancellationToken::new();
    let task = tokio::spawn(
        Verifier::new(pool.clone(), Mode::Repair)
            .with_pace(Duration::from_millis(50))
            .run(stop.clone()),
    );
    let mut left = 1;
    for _ in 0..100 {
        left = count(
            &pool,
            "SELECT count(*) FROM public.repl_checksums WHERE node = 'main'",
        )
        .await;
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the verifier stops on cancel")
        .unwrap()
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM public.repl_repair").await,
        0
    );
}

#[test]
fn mode_parses_the_env_values() {
    assert_eq!("off".parse::<Mode>(), Ok(Mode::Off));
    assert_eq!("dry".parse::<Mode>(), Ok(Mode::Dry));
    assert_eq!("repair".parse::<Mode>(), Ok(Mode::Repair));
    assert!("on".parse::<Mode>().is_err());
}
