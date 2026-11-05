use std::collections::BTreeSet;

use sqlx::PgPool;
use uuid::Uuid;

use super::definer::denied;
use crate::modules::auth::definers::{self, CHAT_FUNCTIONS, OWNER, RELEASE, RELEASE_OWNER};
use crate::modules::auth::test_support::{Restricted, SEALED_FUNCTIONS};

const HELPERS: [&str; 4] = [
    "public.auth_pin_scd()",
    "public.auth_secret(int)",
    "public.auth_start_window(text, int)",
    "public.auth_uuid_v7()",
];
const WRITERS: [&str; 4] = [
    "public.auth_session_end(uuid, bytea)",
    "public.auth_login_create(uuid, bytea, uuid[])",
    "public.auth_link_create(text, uuid, bytea)",
    "public.auth_link_claim(text, uuid, bytea, bytea)",
];
const SEARCH_PATH: &str = "search_path=pg_catalog, public, pg_temp";
const PINNED: [&str; 8] = [
    "TimeZone=UTC",
    "scd.mode=",
    "scd.node=",
    "scd.role=",
    "scd.merging=",
    "scd.cmd=",
    "scd.writer=",
    "scd.guard=",
];

const RESULTS: [(&str, &str); 11] = [
    (
        "public.auth_session(uuid)",
        "TABLE(soundcloud_connection_id uuid, soundcloud_user_id text, chat_bind bytea, created_at timestamp without time zone)",
    ),
    ("public.auth_sessions_alive(bytea[])", "SETOF bytea"),
    ("public.auth_session_logged_out(uuid)", "boolean"),
    ("public.auth_session_username(uuid)", "text"),
    ("public.auth_session_end(uuid, bytea)", "boolean"),
    (
        "public.auth_login_apps()",
        "TABLE(oauth_app_id uuid, client_id text)",
    ),
    (
        "public.auth_login_create(uuid, bytea, uuid[])",
        "TABLE(login_request_id uuid, state text, code_challenge text, client_id text, redirect_uri text)",
    ),
    (
        "public.auth_login_status(uuid)",
        "TABLE(status text, step text, username text, result_session_id uuid, error text, redirect_url text, profile_ok boolean, chat_bind bytea)",
    ),
    (
        "public.auth_link_create(text, uuid, bytea)",
        "TABLE(link_request_id uuid, claim_token text, expires_at timestamp without time zone)",
    ),
    (
        "public.auth_link_claim(text, uuid, bytea, bytea)",
        "TABLE(outcome text, session_id uuid, mode text)",
    ),
    (
        "public.auth_link_status(uuid)",
        "TABLE(status text, mode text, target_session_id uuid, error text, chat_bind bytea)",
    ),
];

const AUTH_GRANTS: [(&str, &str, &str); 21] = [
    ("auth_caps.starts", "DELETE", "*"),
    ("auth_caps.starts", "INSERT", "*"),
    ("auth_caps.starts", "SELECT", "*"),
    (
        "public.link_requests",
        "INSERT",
        "chat_bind,claim_token,expires_at,id,mode,source_session_id,status,target_session_id",
    ),
    ("public.link_requests", "SELECT", "*"),
    (
        "public.link_requests",
        "UPDATE",
        "error,source_session_id,status,target_session_id",
    ),
    (
        "public.login_requests",
        "INSERT",
        "chat_bind,code_verifier,expires_at,id,oauth_app_id,state,status,target_session_id",
    ),
    (
        "public.login_requests",
        "SELECT",
        "chat_bind,error,expires_at,id,profile_ok,redirect_url,result_session_id,status,step,username",
    ),
    (
        "public.oauth_app_request_cooldowns",
        "SELECT",
        "oauth_app_id,retry_at",
    ),
    (
        "public.oauth_apps",
        "SELECT",
        "active,client_id,created_at,id,last_used_at,redirect_uri",
    ),
    ("public.oauth_apps", "UPDATE", "last_used_at,updated_at"),
    ("public.repl_tombstones", "INSERT", "at,pk,tbl,v"),
    ("public.repl_tombstones", "SELECT", "pk,tbl,v"),
    ("public.sessions", "DELETE", "*"),
    (
        "public.sessions",
        "INSERT",
        "chat_bind,id,soundcloud_connection_id",
    ),
    ("public.sessions", "SELECT", "*"),
    ("public.sessions", "UPDATE", "updated_at"),
    (
        "public.soundcloud_connections",
        "SELECT",
        "id,soundcloud_user_id,updated_at,username",
    ),
    (
        "public.sync_queue_action_pauses",
        "SELECT",
        "oauth_app_id,reason",
    ),
    ("schema auth_caps", "USAGE", "*"),
    ("schema public", "USAGE", "*"),
];

const RELEASE_GRANTS: [(&str, &str, &str); 7] = [
    ("public.cmd_outbox", "INSERT", "args,id,kind,node,touched"),
    ("public.repl_purges", "INSERT", "pk,purged_v,tbl"),
    ("public.sessions", "SELECT", "soundcloud_connection_id"),
    ("public.soundcloud_connections", "DELETE", "*"),
    ("public.soundcloud_connections", "SELECT", "id,token_v"),
    ("public.soundcloud_connections", "UPDATE", "updated_at"),
    ("schema public", "USAGE", "*"),
];

const PRIVILEGES: &str = "
    WITH relation AS (
        SELECT class.oid, namespace.nspname || '.' || class.relname AS name, class.relkind
        FROM pg_catalog.pg_class AS class
        JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = class.relnamespace
        WHERE namespace.nspname NOT IN ('pg_catalog', 'information_schema')
          AND namespace.nspname NOT LIKE 'pg\\_%'
          AND class.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
    )
    SELECT relation.name, privilege.name, '*'
    FROM relation,
         unnest(ARRAY['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER', 'MAINTAIN'])
             AS privilege(name)
    WHERE relation.relkind <> 'S'
      AND has_table_privilege($1::name, relation.oid, privilege.name)
    UNION ALL
    SELECT relation.name, privilege.name, string_agg(attribute.attname::text, ',' ORDER BY attribute.attname::text)
    FROM relation
    JOIN pg_catalog.pg_attribute AS attribute
        ON attribute.attrelid = relation.oid AND attribute.attnum > 0 AND NOT attribute.attisdropped
    CROSS JOIN unnest(ARRAY['SELECT', 'INSERT', 'UPDATE', 'REFERENCES']) AS privilege(name)
    WHERE relation.relkind <> 'S'
      AND NOT has_table_privilege($1::name, relation.oid, privilege.name)
      AND has_column_privilege($1::name, relation.oid, attribute.attnum, privilege.name)
    GROUP BY relation.name, privilege.name
    UNION ALL
    SELECT relation.name, privilege.name, '*'
    FROM relation, unnest(ARRAY['USAGE', 'SELECT', 'UPDATE']) AS privilege(name)
    WHERE relation.relkind = 'S'
      AND has_sequence_privilege($1::name, relation.oid, privilege.name)
    UNION ALL
    SELECT 'schema ' || namespace.nspname, privilege.name, '*'
    FROM pg_catalog.pg_namespace AS namespace, unnest(ARRAY['USAGE', 'CREATE']) AS privilege(name)
    WHERE namespace.nspname NOT IN ('pg_catalog', 'information_schema')
      AND namespace.nspname NOT LIKE 'pg\\_%'
      AND has_schema_privilege($1::name, namespace.oid, privilege.name)";

fn expected(grants: &[(&str, &str, &str)]) -> BTreeSet<(String, String, String)> {
    grants
        .iter()
        .map(|(relation, privilege, columns)| {
            (
                (*relation).to_owned(),
                (*privilege).to_owned(),
                (*columns).to_owned(),
            )
        })
        .collect()
}

async fn held(owner: &PgPool, role: &str) -> anyhow::Result<BTreeSet<(String, String, String)>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(PRIVILEGES)
        .bind(role)
        .fetch_all(owner)
        .await?;
    Ok(rows.into_iter().collect())
}

async fn routine(owner: &PgPool, signature: &str) -> anyhow::Result<(String, bool, Vec<String>)> {
    Ok(sqlx::query_as(
        "SELECT proowner::regrole::text, prosecdef, coalesce(proconfig, ARRAY[]::text[])
         FROM pg_catalog.pg_proc
         WHERE oid = $1::text::regprocedure",
    )
    .bind(signature)
    .fetch_one(owner)
    .await?)
}

async fn bare(owner: &PgPool, role: &str) -> anyhow::Result<()> {
    let powers: Option<bool> = sqlx::query_scalar(
        "SELECT rolcanlogin OR rolsuper OR rolinherit OR rolcreaterole OR rolcreatedb
                OR rolreplication OR rolbypassrls OR rolconnlimit <> 0
         FROM pg_catalog.pg_roles
         WHERE rolname = $1",
    )
    .bind(role)
    .fetch_optional(owner)
    .await?;
    assert_eq!(powers, Some(false), "{role} is a bare NOLOGIN role");
    let memberships: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_auth_members
         WHERE $1::text::regrole IN (roleid, member)",
    )
    .bind(role)
    .fetch_one(owner)
    .await?;
    assert_eq!(memberships, 0, "{role} has no role memberships");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn every_auth_function_runs_as_a_bare_nologin_owner(owner: PgPool) -> anyhow::Result<()> {
    bare(&owner, OWNER).await?;
    bare(&owner, RELEASE_OWNER).await?;

    for function in CHAT_FUNCTIONS.into_iter().chain(SEALED_FUNCTIONS) {
        let (owned_by, definer, config) = routine(&owner, function).await?;
        assert_eq!(owned_by, OWNER, "{function}");
        assert!(definer, "{function} is SECURITY DEFINER");
        assert!(
            config.contains(&SEARCH_PATH.to_owned()),
            "{function}: {config:?}"
        );
    }
    for function in WRITERS.into_iter().chain([RELEASE]) {
        let (_, _, config) = routine(&owner, function).await?;
        for setting in PINNED {
            assert!(
                config.contains(&setting.to_owned()),
                "{function} pins {setting}"
            );
        }
    }
    for helper in HELPERS {
        let (owned_by, definer, config) = routine(&owner, helper).await?;
        assert_eq!(owned_by, OWNER, "{helper}");
        assert!(!definer, "{helper} runs as its caller");
        assert!(config.contains(&SEARCH_PATH.to_owned()), "{helper}");
    }
    let (owned_by, definer, config) = routine(&owner, RELEASE).await?;
    assert_eq!(owned_by, RELEASE_OWNER);
    assert!(definer);
    assert!(config.contains(&SEARCH_PATH.to_owned()));

    let executable: Vec<(String, bool, bool)> = sqlx::query_as(
        "SELECT signature,
                has_function_privilege('public', signature, 'EXECUTE'),
                has_function_privilege($1::name, signature, 'EXECUTE')
         FROM unnest($2::text[]) AS signature",
    )
    .bind(OWNER)
    .bind(
        [RELEASE, HELPERS[0], HELPERS[1], HELPERS[2], HELPERS[3]]
            .map(str::to_owned)
            .to_vec(),
    )
    .fetch_all(&owner)
    .await?;
    for (signature, public, by_owner) in executable {
        assert!(!public, "{signature} is closed to PUBLIC");
        assert!(by_owner, "{OWNER} reaches {signature}");
    }
    let release_pins: bool = sqlx::query_scalar(
        "SELECT has_function_privilege($1::name, $2::text::regprocedure, 'EXECUTE')",
    )
    .bind(RELEASE_OWNER)
    .bind(HELPERS[0])
    .fetch_one(&owner)
    .await?;
    assert!(release_pins, "{RELEASE_OWNER} reaches {}", HELPERS[0]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_owner_roles_hold_only_what_the_functions_touch(owner: PgPool) -> anyhow::Result<()> {
    assert_eq!(held(&owner, OWNER).await?, expected(&AUTH_GRANTS));
    assert_eq!(
        held(&owner, RELEASE_OWNER).await?,
        expected(&RELEASE_GRANTS)
    );
    Ok(())
}

async fn probe(owner: &PgPool, name: &str, role: &str, caller: &str) -> anyhow::Result<()> {
    sqlx::raw_sql(&format!(
        "CREATE FUNCTION public.{name}(p_sql text) RETURNS void
         LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public, pg_temp
         AS $f$ BEGIN EXECUTE p_sql; END $f$;
         ALTER FUNCTION public.{name}(text) OWNER TO {role};
         GRANT EXECUTE ON FUNCTION public.{name}(text) TO {caller};"
    ))
    .execute(owner)
    .await?;
    Ok(())
}

async fn refused(pool: &PgPool, name: &str, statements: &[&str]) {
    for statement in statements {
        denied(
            pool,
            &format!("SELECT public.{name}($probe${statement}$probe$)"),
        )
        .await;
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn superuser_powers_stay_out_of_reach_inside_the_auth_functions(
    owner: PgPool,
) -> anyhow::Result<()> {
    let restricted = Restricted::new(&owner).await?;
    let superuser: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&owner)
        .await?;
    probe(&owner, "auth_owner_probe", OWNER, &restricted.role).await?;
    probe(
        &owner,
        "auth_release_probe",
        RELEASE_OWNER,
        &restricted.role,
    )
    .await?;

    let either = [
        "COPY (SELECT 1) TO PROGRAM 'true'".to_owned(),
        "COPY (SELECT 1) TO '/tmp/auth_owner_probe'".to_owned(),
        "SELECT pg_read_file('PG_VERSION')".to_owned(),
        "SELECT pg_ls_dir('.')".to_owned(),
        "SELECT lo_import('/etc/hostname')".to_owned(),
        "SELECT pg_reload_conf()".to_owned(),
        "CREATE ROLE auth_owner_probe_role".to_owned(),
        format!("ALTER ROLE {OWNER} SUPERUSER"),
        format!("ALTER ROLE {RELEASE_OWNER} BYPASSRLS"),
        format!("GRANT {superuser} TO {OWNER}"),
        format!("GRANT {RELEASE_OWNER} TO {OWNER}"),
        format!("SET ROLE {superuser}"),
        "SET session_replication_role = replica".to_owned(),
        "CREATE EXTENSION dblink".to_owned(),
        "LOAD 'auto_explain'".to_owned(),
        format!("ALTER FUNCTION public.auth_session(uuid) OWNER TO {superuser}"),
        "CREATE TABLE public.auth_owner_probe_table (id int)".to_owned(),
        "ALTER TABLE public.sessions DISABLE TRIGGER zz_tomb".to_owned(),
        "TRUNCATE public.sessions".to_owned(),
        "SELECT count(*) FROM public.users".to_owned(),
        "SELECT count(*) FROM chat.devices".to_owned(),
        "SELECT access_token FROM public.soundcloud_connections".to_owned(),
        "SELECT refresh_token FROM public.soundcloud_connections".to_owned(),
        "SELECT client_secret FROM public.oauth_apps".to_owned(),
        "INSERT INTO public.cmd_done (node, id, status) VALUES ('main', gen_random_uuid(), 'ok')"
            .to_owned(),
    ];
    let either: Vec<&str> = either.iter().map(String::as_str).collect();
    refused(&restricted.pool, "auth_owner_probe", &either).await;
    refused(&restricted.pool, "auth_release_probe", &either).await;
    refused(
        &restricted.pool,
        "auth_owner_probe",
        &[
            "DELETE FROM public.soundcloud_connections",
            "SELECT code_verifier FROM public.login_requests",
            "SELECT state FROM public.login_requests",
            "UPDATE public.login_requests SET status = 'failed'",
            "UPDATE public.sessions SET chat_bind = NULL",
            "INSERT INTO public.sessions (id, created_at) VALUES (gen_random_uuid(), now())",
            "UPDATE public.link_requests SET expires_at = now()",
            "INSERT INTO public.cmd_outbox (node, id, kind, args) VALUES ('main', gen_random_uuid(), 'oauth_app.delete', '{}')",
            "INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('sessions', '{}', 0)",
        ],
    )
    .await;
    refused(
        &restricted.pool,
        "auth_release_probe",
        &[
            "SELECT id FROM public.sessions",
            "UPDATE public.sessions SET soundcloud_connection_id = NULL",
            "SELECT access_token FROM public.soundcloud_connections",
            "SELECT count(*) FROM auth_caps.starts",
            "SELECT chat_bind FROM public.sessions",
            "SELECT result_session_id FROM public.login_requests",
            "SELECT claim_token FROM public.link_requests",
            "DELETE FROM public.sessions",
            "INSERT INTO public.sessions (id) VALUES (gen_random_uuid())",
        ],
    )
    .await;

    sqlx::raw_sql(
        "DROP FUNCTION public.auth_owner_probe(text);
         DROP FUNCTION public.auth_release_probe(text);",
    )
    .execute(&owner)
    .await?;
    restricted.remove(&owner).await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_chat_functions_return_only_what_their_callers_hold(
    owner: PgPool,
) -> anyhow::Result<()> {
    let listed: BTreeSet<&str> = RESULTS.iter().map(|(signature, _)| *signature).collect();
    assert_eq!(
        listed,
        CHAT_FUNCTIONS
            .into_iter()
            .chain(SEALED_FUNCTIONS)
            .collect::<BTreeSet<_>>()
    );
    for (signature, result) in RESULTS {
        let actual: String =
            sqlx::query_scalar("SELECT pg_catalog.pg_get_function_result($1::text::regprocedure)")
                .bind(signature)
                .fetch_one(&owner)
                .await?;
        assert_eq!(actual, result, "{signature}");
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn chat_api_refuses_auth_functions_with_another_owner(owner: PgPool) -> anyhow::Result<()> {
    definers::verify(&owner).await?;
    sqlx::raw_sql("ALTER FUNCTION public.auth_session(uuid) OWNER TO CURRENT_USER")
        .execute(&owner)
        .await?;
    let refused = definers::verify(&owner)
        .await
        .expect_err("a superuser owner");
    assert!(
        refused.to_string().contains("public.auth_session(uuid)"),
        "{refused}"
    );
    sqlx::raw_sql("ALTER FUNCTION public.auth_session(uuid) OWNER TO chat_auth")
        .execute(&owner)
        .await?;
    sqlx::raw_sql("ALTER FUNCTION public.auth_connection_release(uuid) OWNER TO chat_auth")
        .execute(&owner)
        .await?;
    let refused = definers::verify(&owner)
        .await
        .expect_err("the wrong helper owner");
    assert!(refused.to_string().contains(RELEASE), "{refused}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn chat_api_refuses_to_start_without_execute_on_a_chat_function(
    owner: PgPool,
) -> anyhow::Result<()> {
    let restricted = Restricted::new(&owner).await?;
    let granted = definers::verify(&restricted.pool).await;
    sqlx::raw_sql(&format!(
        "REVOKE EXECUTE ON FUNCTION public.auth_session_logged_out(uuid) FROM {}",
        restricted.role
    ))
    .execute(&owner)
    .await?;
    let refused = definers::verify(&restricted.pool).await;
    restricted.remove(&owner).await?;

    granted?;
    let refused = refused.expect_err("a missing grant");
    assert!(
        refused
            .to_string()
            .contains("public.auth_session_logged_out(uuid)"),
        "{refused}"
    );
    Ok(())
}

async fn rejected_as_too_long(pool: &PgPool, sql: &str) {
    let error = sqlx::raw_sql(sql).execute(pool).await.expect_err(sql);
    let code = error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned());
    assert_eq!(code.as_deref(), Some("22001"), "{sql}: {error}");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_bind_over_512_bytes_never_reaches_a_table(owner: PgPool) -> anyhow::Result<()> {
    let restricted = Restricted::sealed(&owner).await?;
    let big = "decode(repeat('ab', 513), 'hex')";
    rejected_as_too_long(
        &restricted.pool,
        &format!("SELECT * FROM public.auth_link_create('pull', NULL, {big})"),
    )
    .await;
    rejected_as_too_long(
        &restricted.pool,
        &format!("SELECT * FROM public.auth_login_create(NULL, {big}, '{{}}')"),
    )
    .await;
    rejected_as_too_long(
        &restricted.pool,
        &format!("SELECT * FROM public.auth_link_claim('token', NULL, {big}, NULL)"),
    )
    .await;
    rejected_as_too_long(
        &restricted.pool,
        &format!("SELECT * FROM public.auth_link_claim('token', NULL, NULL, {big})"),
    )
    .await;
    let fits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.auth_link_create('pull', NULL, decode(repeat('ab', 512), 'hex'))",
    )
    .fetch_one(&restricted.pool)
    .await?;
    assert_eq!(fits, 1);
    restricted.remove(&owner).await
}

async fn refused_by_check(owner: &PgPool, sql: &str) {
    let error = sqlx::raw_sql(sql).execute(owner).await.expect_err(sql);
    let code = error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned());
    assert_eq!(code.as_deref(), Some("23514"), "{sql}: {error}");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_main_api_cannot_store_a_bind_over_512_bytes_either(
    owner: PgPool,
) -> anyhow::Result<()> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, 'big-bind', 'big-bind', 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .execute(&owner)
    .await?;
    let big = vec![7u8; 513];
    let stored = sqlx::query_file!(
        "queries/auth/service/insert_session.sql",
        Uuid::now_v7(),
        connection,
        Some(big.as_slice())
    )
    .fetch_one(&owner)
    .await
    .expect_err("an oversized bind");
    let code = stored
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned());
    assert_eq!(code.as_deref(), Some("23514"), "{stored}");
    let big = "decode(repeat('ab', 513), 'hex')";
    refused_by_check(
        &owner,
        &format!(
            "INSERT INTO login_requests (id, state, code_verifier, expires_at, chat_bind)
             VALUES (gen_random_uuid(), 'big-bind', 'verifier', now(), {big})"
        ),
    )
    .await;
    refused_by_check(
        &owner,
        &format!(
            "INSERT INTO link_requests (id, claim_token, mode, status, expires_at, chat_bind)
             VALUES (gen_random_uuid(), 'big-bind', 'pull', 'pending', now(), {big})"
        ),
    )
    .await;
    let fits = vec![7u8; 512];
    sqlx::query_file!(
        "queries/auth/service/insert_session.sql",
        Uuid::now_v7(),
        connection,
        Some(fits.as_slice())
    )
    .fetch_one(&owner)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn every_pinned_search_path_ends_with_pg_temp(owner: PgPool) -> anyhow::Result<()> {
    let open: Vec<String> = sqlx::query_scalar(
        "SELECT routine.oid::regprocedure::text || ' ' || config
         FROM pg_catalog.pg_proc AS routine,
              unnest(routine.proconfig) AS config
         WHERE routine.pronamespace NOT IN ('pg_catalog'::regnamespace, 'information_schema'::regnamespace)
           AND config LIKE 'search_path=%'
           AND config NOT LIKE '%, pg\\_temp'
         ORDER BY 1",
    )
    .fetch_all(&owner)
    .await?;
    assert_eq!(open, Vec::<String>::new());

    let definers: Vec<(String, String)> = sqlx::query_as(
        "SELECT routine.oid::regprocedure::text, routine.prorettype::regtype::text
         FROM pg_catalog.pg_proc AS routine
         WHERE routine.prosecdef
           AND routine.pronamespace NOT IN ('pg_catalog'::regnamespace, 'information_schema'::regnamespace)
           AND has_function_privilege('public', routine.oid, 'EXECUTE')
         ORDER BY 1",
    )
    .fetch_all(&owner)
    .await?;
    assert_eq!(
        definers,
        vec![("scd_dispatch()".to_owned(), "trigger".to_owned())],
        "PUBLIC reaches no SECURITY DEFINER function it can call"
    );

    let database_grants: Vec<String> = sqlx::query_scalar(
        "SELECT item.grantee::regrole::text || ' ' || item.privilege_type
         FROM pg_catalog.pg_database AS database,
              aclexplode(database.datacl) AS item
         WHERE database.datname = current_database()
           AND item.grantee IN ($1::text::regrole, $2::text::regrole)",
    )
    .bind(OWNER)
    .bind(RELEASE_OWNER)
    .fetch_all(&owner)
    .await?;
    assert_eq!(database_grants, Vec::<String>::new());
    Ok(())
}
