use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;

use repl_verify::{
    Mode, Sweeper, Verifier, api_node, probe_once, repair_once, retain_once, sweep_once,
};
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
    assert!(
        rows[0].stale_hash() && rows[0].attempt == 0,
        "an answer without hv comes from a node on the md5 hash: {rows:?}"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM public.repl_checksums WHERE node = 'star'"
        )
        .await,
        0
    );
    sqlx::query(
        "INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
        VALUES ('star', 'artists', gen_random_uuid()::text,
                jsonb_build_object('hv', 2, 'lo', NULL, 'hi', NULL, 'attempt', 0, 'n', 1, 'keys', NULL)::text, now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    let rows = repair_once(&pool, false, 50).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].recheck() && rows[0].attempt == 0, "{rows:?}");
    let answer ="INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
        VALUES ('main', 'artists', gen_random_uuid()::text,
                jsonb_build_object('hv', 2, 'lo', NULL, 'hi', NULL, 'attempt', 1, 'n', 1,
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

const OUTBOX: &str = "INSERT INTO public.cmd_outbox (node, id, kind, args, created_at, done_at) VALUES
    ('main', '00000000-0000-4000-8000-0000000000c1', 'track.edit', '{}', now() - interval '3 days', now() - interval '3 days'),
    ('main', '00000000-0000-4000-8000-0000000000c2', 'track.edit', '{}', now() - interval '3 days', now() - interval '1 day'),
    ('main', '00000000-0000-4000-8000-0000000000c3', 'track.edit', '{}', now() - interval '30 days', NULL),
    ('main', '00000000-0000-4000-8000-0000000000c4', 'connection.login', '{\"access_token\": \"a\", \"refresh_token\": \"r\", \"login_v\": 1}', now() - interval '3 hours', now() - interval '2 hours'),
    ('main', '00000000-0000-4000-8000-0000000000c5', 'connection.login', '{\"access_token\": \"a\", \"refresh_token\": \"r\", \"login_v\": 2}', now() - interval '20 minutes', now() - interval '10 minutes'),
    ('main', '00000000-0000-4000-8000-0000000000c6', 'connection.login', '{\"access_token\": \"a\", \"refresh_token\": \"r\", \"login_v\": 3}', now() - interval '1 day', NULL),
    ('main', '00000000-0000-4000-8000-0000000000c7', 'connection.login', '{\"access_token\": \"a\", \"refresh_token\": \"r\", \"login_v\": 4}', now() - interval '8 days', NULL);
    INSERT INTO public.oauth_app_tokens_local (oauth_app_id, access_token, expires_at) VALUES
    ('40000000-0000-4000-8000-0000000000d1', 'gone', now() - interval '2 hours'),
    ('40000000-0000-4000-8000-0000000000d2', 'fresh-expired', now() - interval '10 minutes'),
    ('40000000-0000-4000-8000-0000000000d3', 'valid', now() + interval '1 hour')";

async fn outbox(pool: &PgPool) -> Vec<(String, bool)> {
    sqlx::query_as(
        "SELECT right(id::text, 2), args ?| ARRAY['access_token', 'refresh_token'] FROM public.cmd_outbox ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn node_retention_drops_old_acked_commands_and_redacts_logins_but_keeps_pending_ones() {
    let pool = node("retain", None).await;
    sqlx::raw_sql(OUTBOX).execute(&pool).await.unwrap();

    let first = retain_once(&pool, "cmd_outbox", 1000).await.unwrap();
    assert_eq!((first.deleted, first.redacted), (1, 2), "{first:?}");
    assert_eq!(
        first.held.as_deref(),
        Some("2 commands wait for core longer than 2 days")
    );
    assert_eq!(first.reason.as_deref(), Some("pending"));
    let left = outbox(&pool).await;
    let expected = [
        ("c2", false),
        ("c3", false),
        ("c4", false),
        ("c5", true),
        ("c6", true),
        ("c7", false),
    ];
    assert_eq!(
        left,
        expected
            .map(|(id, tokens)| (id.to_owned(), tokens))
            .to_vec()
    );
    let again = retain_once(&pool, "cmd_outbox", 1000).await.unwrap();
    assert_eq!((again.deleted, again.redacted), (0, 0), "{again:?}");

    let tokens = retain_once(&pool, "oauth_app_tokens_local", 1000)
        .await
        .unwrap();
    assert_eq!((tokens.deleted, tokens.held), (1, None));
    assert_eq!(
        count(&pool, "SELECT count(*) FROM public.oauth_app_tokens_local").await,
        2
    );
}

#[tokio::test]
async fn an_api_node_without_its_core_stream_holds_node_retention() {
    let pool = node("retain_api", Some(("main", "api"))).await;
    sqlx::raw_sql(OUTBOX).execute(&pool).await.unwrap();

    let held = retain_once(&pool, "cmd_outbox", 1000).await.unwrap();
    assert_eq!((held.deleted, held.redacted), (0, 0), "{held:?}");
    assert_eq!(
        held.held.as_deref(),
        Some("slots: no slot for core; subscriptions: no subscription from core")
    );
    assert_eq!(held.reason.as_deref(), Some("subscription"));
    assert_eq!(
        count(&pool, "SELECT count(*) FROM public.cmd_outbox").await,
        7
    );
    let tokens = retain_once(&pool, "oauth_app_tokens_local", 1000)
        .await
        .unwrap();
    assert_eq!(
        tokens.deleted, 1,
        "expired tokens are not replicated and go anyway"
    );
}

const RV_T1: &str = "{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}";
const RV_T2: &str = "{\"user_id\": \"rv\", \"sc_track_id\": \"t2\"}";
const RV_T3: &str = "{\"user_id\": \"rv\", \"sc_track_id\": \"t3\"}";
const OLD_RECEIPT: &str = "INSERT INTO public.repl_purges (tbl, pk, purged_v, at)
    VALUES ('user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}', 0, now() - interval '8 days')";

#[tokio::test]
async fn receipt_retention_runs_on_api_nodes_only_and_waits_for_the_core_stream() {
    let cases = [
        ("receipts_legacy", None, false, None),
        ("receipts_core", Some(("core", "core")), false, None),
        (
            "receipts_api",
            Some(("main", "api")),
            true,
            Some("slots: no slot for core; subscriptions: no subscription from core"),
        ),
    ];
    for (case, configure, api, held) in cases {
        let pool = node(case, configure).await;
        sqlx::raw_sql(OLD_RECEIPT).execute(&pool).await.unwrap();
        assert_eq!(api_node(&pool).await.unwrap(), api, "{case}");
        let kept = retain_once(&pool, "repl_purges", 1000).await.unwrap();
        assert_eq!(
            (kept.deleted, kept.redacted, kept.held.as_deref()),
            (0, 0, held),
            "{case}"
        );
        assert_eq!(
            count(&pool, "SELECT count(*) FROM public.repl_purges").await,
            1,
            "{case}"
        );
    }
}

#[tokio::test]
async fn core_sends_its_receipts_before_the_image_when_the_api_holds_others() {
    let pool = node("receipts_settle", Some(("core", "core"))).await;
    sqlx::raw_sql(
        "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES
             ('user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t3\"}', 16, '2026-11-01 05:28:38.123456+00');
         INSERT INTO public.user_likes_tracks (user_id, sc_track_id, wanted_state, wanted_v) VALUES
             ('rv', 't1', true, 0), ('rv', 't2', true, 0), ('rv', 't3', true, 32);
         INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
         VALUES ('main', 'user_likes_tracks', gen_random_uuid()::text,
                 jsonb_build_object('hv', 2, 'lo', NULL, 'hi', NULL, 'attempt', 1, 'n', 0,
                     'keys', jsonb_build_object(
                         '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}', NULL,
                         '{\"user_id\": \"rv\", \"sc_track_id\": \"t2\"}', NULL,
                         '{\"user_id\": \"rv\", \"sc_track_id\": \"t3\"}', NULL),
                     'receipts', jsonb_build_object(
                         '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}',
                         jsonb_build_array(jsonb_build_object('purged_v', 0, 'at', '2026-11-01T05:28:38+00:00')),
                         '{\"user_id\": \"rv\", \"sc_track_id\": \"t3\"}',
                         jsonb_build_array(jsonb_build_object('purged_v', 16, 'at', '2026-11-01T08:28:38.123456+03:00'))))::text,
                 now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    let rows = repair_once(&pool, true, 50).await.unwrap();
    let acts: Vec<(&str, &str, i32)> = rows
        .iter()
        .map(|r| (r.k.as_deref().unwrap_or("-"), r.action.as_str(), r.attempt))
        .collect();
    assert_eq!(
        acts,
        [
            (RV_T1, "receipts", 1),
            (RV_T1, "row", 1),
            (RV_T2, "row", 1),
            (RV_T3, "row", 1),
        ]
    );
}

#[tokio::test]
async fn an_api_node_keeps_exactly_cores_receipts_for_a_repaired_key() {
    let pool = node("receipts_take", Some(("main", "api"))).await;
    sqlx::raw_sql(
        "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES
             ('user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}', 0, '2026-11-01 05:28:38+00'),
             ('user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}', 32, '2026-11-02 00:00:00+00'),
             ('user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t2\"}', 0, '2026-11-01 05:28:38+00');
         SELECT public.scd_repair_op(gen_random_uuid(), 'user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}',
             '{\"scd_op\": \"receipts\", \"receipts\": [{\"purged_v\": 32, \"at\": \"2026-11-02T03:00:00+03:00\"}, {\"purged_v\": 48, \"at\": \"2026-11-03T00:00:00+00:00\"}]}');
         SELECT public.scd_repair_op(gen_random_uuid(), 'artists', '{\"id\": \"00000000-0000-0000-0000-0000000000e1\"}',
             '{\"scd_op\": \"receipts\", \"receipts\": []}');",
    )
    .execute(&pool)
    .await
    .unwrap();
    let left: Vec<(String, i64)> = sqlx::query_as(
        "SELECT pk ->> 'sc_track_id', purged_v FROM public.repl_purges ORDER BY 1, 2",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        left,
        [("t1", 32), ("t1", 48), ("t2", 0)]
            .map(|(track, v)| (track.to_owned(), v))
            .to_vec()
    );

    let core = node("receipts_take_core", Some(("core", "core"))).await;
    sqlx::raw_sql(
        "INSERT INTO public.repl_purges (tbl, pk, purged_v, at) VALUES
             ('user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}', 0, '2026-11-01 05:28:38+00');
         SELECT public.scd_repair_op(gen_random_uuid(), 'user_likes_tracks', '{\"user_id\": \"rv\", \"sc_track_id\": \"t1\"}',
             '{\"scd_op\": \"receipts\", \"receipts\": []}');",
    )
    .execute(&core)
    .await
    .unwrap();
    assert_eq!(
        count(&core, "SELECT count(*) FROM public.repl_purges").await,
        1,
        "core never takes receipts from a repair op"
    );
}
