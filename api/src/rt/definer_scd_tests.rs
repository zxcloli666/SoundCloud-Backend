use sqlx::postgres::PgPoolOptions;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::definer::{connected, denied};
use crate::modules::auth::test_support::Restricted;

async fn node(owner: &PgPool, mode: &str, role: &str, node: &str) -> anyhow::Result<()> {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(owner)
        .await?;
    sqlx::raw_sql(&format!(
        "ALTER DATABASE \"{database}\" SET scd.mode = '{mode}';
         ALTER DATABASE \"{database}\" SET scd.role = '{role}';
         ALTER DATABASE \"{database}\" SET scd.node = '{node}';"
    ))
    .execute(owner)
    .await?;
    Ok(())
}

async fn tombstone(owner: &PgPool, session: Uuid) -> anyhow::Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT max(v & 15) FROM repl_tombstones
         WHERE tbl = 'sessions' AND pk = jsonb_build_object('id', $1::uuid)",
    )
    .bind(session)
    .fetch_one(owner)
    .await?)
}

async fn connection_of(owner: &PgPool, session: Uuid) -> anyhow::Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT soundcloud_connection_id FROM sessions WHERE id = $1")
            .bind(session)
            .fetch_one(owner)
            .await?,
    )
}

async fn end(pool: &PgPool, session: Uuid) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT public.auth_session_end($1, sha256(uuid_send($1)))")
            .bind(session)
            .fetch_one(pool)
            .await?,
    )
}

async fn link_node(caller: &mut PgConnection, owner: &PgPool) -> anyhow::Result<i64> {
    let link: Uuid = sqlx::query_scalar(
        "SELECT link_request_id FROM public.auth_link_create('pull', NULL, NULL)",
    )
    .fetch_one(&mut *caller)
    .await?;
    Ok(
        sqlx::query_scalar("SELECT row_v & 15 FROM link_requests WHERE id = $1")
            .bind(link)
            .fetch_one(owner)
            .await?,
    )
}

async fn written_nodes(pool: &PgPool, owner: &PgPool, source: Uuid) -> anyhow::Result<Vec<i64>> {
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'node', $2, 'secret', 'https://app.test/')
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::from_u128(0x5cd))
    .bind("client-node")
    .execute(owner)
    .await?;
    let (link, token): (Uuid, String) = sqlx::query_as(
        "SELECT link_request_id, claim_token FROM public.auth_link_create('pull', NULL, '\\x01')",
    )
    .fetch_one(pool)
    .await?;
    let (outcome, session): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT outcome, session_id FROM public.auth_link_claim($1, $2, NULL, NULL)",
    )
    .bind(&token)
    .bind(source)
    .fetch_one(pool)
    .await?;
    assert_eq!(outcome, "claimed");
    let login: Uuid = sqlx::query_scalar(
        "SELECT login_request_id FROM public.auth_login_create(NULL, '\\x01', '{}')",
    )
    .fetch_one(pool)
    .await?;
    Ok(sqlx::query_scalar(
        "SELECT row_v & 15 FROM sessions WHERE id = $1
         UNION ALL SELECT row_v & 15 FROM link_requests WHERE id = $2
         UNION ALL SELECT row_v & 15 FROM login_requests WHERE id = $3",
    )
    .bind(session)
    .bind(link)
    .bind(login)
    .fetch_all(owner)
    .await?)
}

async fn purged(owner: &PgPool, connection: Uuid) -> anyhow::Result<(bool, bool)> {
    let left: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM soundcloud_connections WHERE id = $1)")
            .bind(connection)
            .fetch_one(owner)
            .await?;
    let recorded: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM repl_purges
             WHERE tbl = 'soundcloud_connections' AND pk = jsonb_build_object('id', $1::uuid)
         )",
    )
    .bind(connection)
    .fetch_one(owner)
    .await?;
    Ok((left, recorded))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_caller_cannot_steer_replication_from_inside_a_definer_function(
    owner: PgPool,
) -> anyhow::Result<()> {
    node(&owner, "split", "api", "main").await?;
    let restricted = Restricted::new(&owner).await?;
    let session = connected(&owner, "steer", None).await?;
    let connection = connection_of(&owner, session).await?;

    let mut caller = restricted.pool.acquire().await?;
    sqlx::raw_sql(
        "SET scd.mode = 'legacy';
         SET scd.node = 'core';
         SET scd.role = 'core';
         SET scd.merging = 'on';
         SET scd.cmd = 'repl';",
    )
    .execute(&mut *caller)
    .await?;

    let ended: bool =
        sqlx::query_scalar("SELECT public.auth_session_end($1, sha256(uuid_send($1)))")
            .bind(session)
            .fetch_one(&mut *caller)
            .await?;
    assert!(ended);
    assert_eq!(
        tombstone(&owner, session).await?,
        Some(2),
        "split mode writes the tombstone with the node of the database"
    );
    let shipped: Option<String> = sqlx::query_scalar(
        "SELECT node FROM cmd_outbox
         WHERE kind = 'connection.drop_orphan' AND args = jsonb_build_object('connection_id', $1::uuid)",
    )
    .bind(connection)
    .fetch_optional(&owner)
    .await?;
    assert_eq!(
        shipped.as_deref(),
        Some("main"),
        "an api node ships the orphan drop to core"
    );
    assert_eq!(
        purged(&owner, connection).await?,
        (true, false),
        "an api node leaves the purge to core"
    );

    let link: Uuid = sqlx::query_scalar(
        "SELECT link_request_id FROM public.auth_link_create('pull', NULL, NULL)",
    )
    .fetch_one(&mut *caller)
    .await?;
    let stamped: i64 = sqlx::query_scalar("SELECT row_v & 15 FROM link_requests WHERE id = $1")
        .bind(link)
        .fetch_one(&owner)
        .await?;
    assert_eq!(
        stamped, 2,
        "the version carries the node of the database, not the caller"
    );

    sqlx::raw_sql("BEGIN; SET LOCAL scd.mode = 'merge'")
        .execute(&mut *caller)
        .await?;
    sqlx::query("SELECT * FROM public.auth_link_create('pull', NULL, NULL)")
        .execute(&mut *caller)
        .await?;
    let kept: String = sqlx::query_scalar("SELECT current_setting('scd.mode')")
        .fetch_one(&mut *caller)
        .await?;
    sqlx::raw_sql("ROLLBACK").execute(&mut *caller).await?;
    assert_eq!(
        kept, "merge",
        "the call gives the caller its own setting back"
    );

    let source = connected(&owner, "steer-source", None).await?;
    assert_eq!(
        written_nodes(&restricted.pool, &owner, source).await?,
        [2, 2, 2],
        "claims and logins stamp the node of the database"
    );

    denied(&restricted.pool, "SELECT public.auth_pin_scd()").await;
    drop(caller);
    restricted.remove(&owner).await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn core_purges_the_orphaned_connection_of_an_ended_session(
    owner: PgPool,
) -> anyhow::Result<()> {
    node(&owner, "split", "core", "core").await?;
    let restricted = Restricted::new(&owner).await?;
    let session = connected(&owner, "core", None).await?;
    let connection = connection_of(&owner, session).await?;
    let kept = connected(&owner, "core-kept", None).await?;
    let shared = connection_of(&owner, kept).await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(Uuid::now_v7())
        .bind(shared)
        .execute(&owner)
        .await?;

    assert!(end(&restricted.pool, session).await?);
    assert_eq!(tombstone(&owner, session).await?, Some(1));
    assert_eq!(
        purged(&owner, connection).await?,
        (false, true),
        "core purges the connection and records the purge"
    );
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM cmd_outbox")
        .fetch_one(&owner)
        .await?;
    assert_eq!(outbox, 0, "core runs the command in place");

    assert!(end(&restricted.pool, kept).await?);
    assert_eq!(
        purged(&owner, shared).await?,
        (true, false),
        "a connection another session still holds stays"
    );
    let mut caller = restricted.pool.acquire().await?;
    assert_eq!(link_node(&mut caller, &owner).await?, 1);
    drop(caller);
    let source = connected(&owner, "core-source", None).await?;
    assert_eq!(
        written_nodes(&restricted.pool, &owner, source).await?,
        [1, 1, 1]
    );

    restricted.remove(&owner).await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_legacy_node_drops_the_orphaned_connection_in_place(owner: PgPool) -> anyhow::Result<()> {
    let restricted = Restricted::new(&owner).await?;
    let session = connected(&owner, "legacy", None).await?;
    let connection = connection_of(&owner, session).await?;

    assert!(end(&restricted.pool, session).await?);
    assert!(!end(&restricted.pool, session).await?);
    assert_eq!(
        tombstone(&owner, session).await?,
        None,
        "legacy mode writes no tombstones"
    );
    assert_eq!(purged(&owner, connection).await?, (false, true));
    let mut caller = restricted.pool.acquire().await?;
    sqlx::raw_sql(
        "SET scd.mode = 'split';
         SET scd.role = 'api';
         SET scd.node = 'star';",
    )
    .execute(&mut *caller)
    .await?;
    assert_eq!(
        link_node(&mut caller, &owner).await?,
        1,
        "the caller's own scd settings do not reach the definer"
    );
    drop(caller);

    restricted.remove(&owner).await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_node_setting_on_the_role_stops_the_writers(owner: PgPool) -> anyhow::Result<()> {
    let restricted = Restricted::new(&owner).await?;
    let mut codes = Vec::new();
    for (name, value) in [("scd.mode", "split"), ("scd.guard", "log")] {
        sqlx::raw_sql(&format!(
            "ALTER ROLE {} SET {name} = '{value}'",
            restricted.role
        ))
        .execute(&owner)
        .await?;
        let configured = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(restricted.pool.connect_options().as_ref().clone())
            .await?;
        let error = sqlx::query_scalar::<_, Uuid>(
            "SELECT link_request_id FROM public.auth_link_create('pull', NULL, NULL)",
        )
        .fetch_one(&configured)
        .await
        .expect_err("a node setting outside ALTER DATABASE");
        configured.close().await;
        sqlx::raw_sql(&format!("ALTER ROLE {} RESET {name}", restricted.role))
            .execute(&owner)
            .await?;
        let code = error
            .as_database_error()
            .and_then(|error| error.code())
            .map(|code| code.into_owned());
        codes.push((name, code));
    }
    restricted.remove(&owner).await?;

    assert_eq!(
        codes,
        [
            ("scd.mode", Some("55000".to_owned())),
            ("scd.guard", Some("55000".to_owned()))
        ]
    );
    Ok(())
}

async fn watch_writes(owner: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE public.definer_writes (tbl text, writer text, guard text);
         GRANT INSERT ON public.definer_writes TO PUBLIC;
         CREATE FUNCTION public.definer_writes_seen() RETURNS trigger
         LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp
         AS $f$
         BEGIN
             INSERT INTO public.definer_writes
             VALUES (TG_TABLE_NAME, current_setting('scd.writer', true), current_setting('scd.guard', true));
             RETURN coalesce(NEW, OLD);
         END
         $f$;
         CREATE TRIGGER zz_seen BEFORE INSERT OR UPDATE OR DELETE ON public.sessions
             FOR EACH ROW EXECUTE FUNCTION public.definer_writes_seen();
         CREATE TRIGGER zz_seen BEFORE INSERT OR UPDATE ON public.link_requests
             FOR EACH ROW EXECUTE FUNCTION public.definer_writes_seen();
         CREATE TRIGGER zz_seen BEFORE INSERT ON public.login_requests
             FOR EACH ROW EXECUTE FUNCTION public.definer_writes_seen();
         CREATE TRIGGER zz_seen BEFORE INSERT ON public.cmd_outbox
             FOR EACH ROW EXECUTE FUNCTION public.definer_writes_seen();",
    )
    .execute(owner)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn definer_writes_run_with_no_writer_and_the_guard_of_the_database(
    owner: PgPool,
) -> anyhow::Result<()> {
    node(&owner, "split", "api", "main").await?;
    sqlx::raw_sql(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET scd.guard = %L', current_database(), 'enforce'); END $$",
    )
    .execute(&owner)
    .await?;
    let restricted = Restricted::new(&owner).await?;
    let source = connected(&owner, "writer-source", None).await?;
    let ended = connected(&owner, "writer-ended", None).await?;
    watch_writes(&owner).await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            restricted
                .pool
                .connect_options()
                .as_ref()
                .clone()
                .options([("scd.writer", "repl"), ("scd.guard", "log")]),
        )
        .await?;

    written_nodes(&pool, &owner, source).await?;
    assert!(end(&pool, ended).await?);
    let kept: (String, String) =
        sqlx::query_as("SELECT current_setting('scd.writer'), current_setting('scd.guard')")
            .fetch_one(&pool)
            .await?;
    pool.close().await;
    restricted.remove(&owner).await?;

    let seen: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT DISTINCT tbl, writer, guard FROM public.definer_writes ORDER BY tbl",
    )
    .fetch_all(&owner)
    .await?;
    let expected = |tbl: &str| (tbl.to_owned(), String::new(), "enforce".to_owned());
    assert_eq!(
        seen,
        [
            expected("cmd_outbox"),
            expected("link_requests"),
            expected("login_requests"),
            expected("sessions"),
        ]
    );
    assert_eq!(kept, ("repl".to_owned(), "log".to_owned()));
    Ok(())
}
