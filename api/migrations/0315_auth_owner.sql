SET LOCAL lock_timeout = '5s';

DO $do$
DECLARE
    v_attributes constant text :=
        'NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 0';
    v_database oid := (SELECT database.oid FROM pg_catalog.pg_database AS database WHERE database.datname = current_database());
    v_role text;
    v_statement text;
    v_leftover text;
BEGIN
    FOREACH v_role IN ARRAY ARRAY['chat_auth', 'chat_auth_release'] LOOP
        BEGIN
            EXECUTE format('CREATE ROLE %I %s', v_role, v_attributes);
        EXCEPTION WHEN duplicate_object OR unique_violation THEN
            NULL;
        END;
        IF EXISTS (
            SELECT 1
            FROM pg_catalog.pg_roles AS role
            WHERE role.rolname = v_role
              AND (role.rolcanlogin OR role.rolsuper OR role.rolcreatedb OR role.rolcreaterole OR role.rolinherit
                   OR role.rolreplication OR role.rolbypassrls OR role.rolconnlimit <> 0)
        ) THEN
            EXECUTE format('ALTER ROLE %I %s PASSWORD NULL', v_role, v_attributes);
        END IF;
        FOR v_statement IN
            SELECT format('REVOKE %s FROM %s GRANTED BY %s',
                          membership.roleid::regrole, membership.member::regrole, membership.grantor::regrole)
            FROM pg_catalog.pg_auth_members AS membership
            WHERE v_role::regrole IN (membership.roleid, membership.member)
            UNION ALL
            SELECT format('REVOKE ALL ON %s %s FROM %I',
                          CASE relation.relkind WHEN 'S' THEN 'SEQUENCE' ELSE 'TABLE' END, relation.oid::regclass, v_role)
            FROM pg_catalog.pg_class AS relation
            WHERE v_role::regrole IN (SELECT item.grantee FROM pg_catalog.aclexplode(relation.relacl) AS item)
               OR v_role::regrole IN (
                      SELECT item.grantee
                      FROM pg_catalog.pg_attribute AS attribute,
                           pg_catalog.aclexplode(attribute.attacl) AS item
                      WHERE attribute.attrelid = relation.oid
                  )
            UNION ALL
            SELECT format('REVOKE ALL ON ROUTINE %s FROM %I', routine.oid::regprocedure, v_role)
            FROM pg_catalog.pg_proc AS routine
            WHERE routine.proowner <> v_role::regrole
              AND v_role::regrole IN (SELECT item.grantee FROM pg_catalog.aclexplode(routine.proacl) AS item)
            UNION ALL
            SELECT format('REVOKE ALL ON SCHEMA %I FROM %I', namespace.nspname, v_role)
            FROM pg_catalog.pg_namespace AS namespace
            WHERE v_role::regrole IN (SELECT item.grantee FROM pg_catalog.aclexplode(namespace.nspacl) AS item)
            UNION ALL
            SELECT format('REVOKE ALL ON DATABASE %I FROM %I', database.datname, v_role)
            FROM pg_catalog.pg_database AS database
            WHERE database.oid = v_database
              AND v_role::regrole IN (SELECT item.grantee FROM pg_catalog.aclexplode(database.datacl) AS item)
            UNION ALL
            SELECT format('REVOKE ALL ON PARAMETER %s FROM %I', parameter.parname, v_role)
            FROM pg_catalog.pg_parameter_acl AS parameter
            WHERE v_role::regrole IN (SELECT item.grantee FROM pg_catalog.aclexplode(parameter.paracl) AS item)
        LOOP
            EXECUTE v_statement;
        END LOOP;
        SELECT string_agg(pg_catalog.pg_describe_object(dependency.classid, dependency.objid, dependency.objsubid)
                              || ' (' || dependency.deptype::text || ')', ', ')
        INTO v_leftover
        FROM pg_catalog.pg_shdepend AS dependency
        WHERE dependency.refclassid = 'pg_catalog.pg_authid'::regclass
          AND dependency.refobjid = v_role::regrole
          AND (dependency.dbid = v_database
               OR (dependency.dbid = 0
                   AND (dependency.classid <> 'pg_catalog.pg_database'::regclass OR dependency.objid = v_database)))
          AND NOT (dependency.classid = 'pg_catalog.pg_proc'::regclass
                   AND dependency.deptype = 'o'
                   AND dependency.objid IN (
                           SELECT routine.oid
                           FROM pg_catalog.pg_proc AS routine
                           WHERE routine.pronamespace = 'public'::regnamespace
                             AND routine.proname LIKE 'auth\_%'
                       ));
        IF v_leftover IS NOT NULL THEN
            RAISE EXCEPTION 'role % still holds %', v_role, v_leftover
                USING HINT = 'remove these grants, policies, default privileges or objects by hand, then rerun the core migrations';
        END IF;
    END LOOP;
END
$do$;

GRANT SELECT, DELETE, INSERT (id, soundcloud_connection_id, chat_bind), UPDATE (updated_at)
    ON public.sessions TO chat_auth;
GRANT SELECT (id, status, step, username, result_session_id, error, redirect_url, profile_ok, chat_bind, expires_at),
      INSERT (id, state, code_verifier, oauth_app_id, target_session_id, status, expires_at, chat_bind)
    ON public.login_requests TO chat_auth;
GRANT SELECT,
      INSERT (id, claim_token, mode, source_session_id, target_session_id, status, expires_at, chat_bind),
      UPDATE (status, error, source_session_id, target_session_id)
    ON public.link_requests TO chat_auth;
GRANT SELECT (tbl, pk, v), INSERT (tbl, pk, v, at) ON public.repl_tombstones TO chat_auth;
GRANT SELECT (id, soundcloud_user_id, username, updated_at) ON public.soundcloud_connections TO chat_auth;
GRANT SELECT (id, active, client_id, redirect_uri, last_used_at, created_at), UPDATE (last_used_at, updated_at)
    ON public.oauth_apps TO chat_auth;
GRANT SELECT (oauth_app_id, retry_at) ON public.oauth_app_request_cooldowns TO chat_auth;
GRANT SELECT (oauth_app_id, reason) ON public.sync_queue_action_pauses TO chat_auth;
GRANT USAGE ON SCHEMA auth_caps TO chat_auth;
GRANT SELECT, INSERT, DELETE ON auth_caps.starts TO chat_auth;

GRANT SELECT (id, token_v), UPDATE (updated_at), DELETE ON public.soundcloud_connections TO chat_auth_release;
GRANT SELECT (soundcloud_connection_id) ON public.sessions TO chat_auth_release;
GRANT INSERT (tbl, pk, purged_v) ON public.repl_purges TO chat_auth_release;
GRANT INSERT (node, id, kind, args, touched) ON public.cmd_outbox TO chat_auth_release;

ALTER FUNCTION public.auth_session(uuid) OWNER TO chat_auth;
ALTER FUNCTION public.auth_sessions_alive(bytea[]) OWNER TO chat_auth;
ALTER FUNCTION public.auth_session_logged_out(uuid) OWNER TO chat_auth;
ALTER FUNCTION public.auth_session_username(uuid) OWNER TO chat_auth;
ALTER FUNCTION public.auth_session_end(uuid, bytea) OWNER TO chat_auth;
ALTER FUNCTION public.auth_login_apps() OWNER TO chat_auth;
ALTER FUNCTION public.auth_login_create(uuid, bytea, uuid[]) OWNER TO chat_auth;
ALTER FUNCTION public.auth_login_status(uuid) OWNER TO chat_auth;
ALTER FUNCTION public.auth_link_create(text, uuid, bytea) OWNER TO chat_auth;
ALTER FUNCTION public.auth_link_claim(text, uuid, bytea, bytea) OWNER TO chat_auth;
ALTER FUNCTION public.auth_link_status(uuid) OWNER TO chat_auth;
ALTER FUNCTION public.auth_pin_scd() OWNER TO chat_auth;
ALTER FUNCTION public.auth_secret(int) OWNER TO chat_auth;
ALTER FUNCTION public.auth_start_window(text, int) OWNER TO chat_auth;
ALTER FUNCTION public.auth_uuid_v7() OWNER TO chat_auth;
ALTER FUNCTION public.auth_connection_release(uuid) OWNER TO chat_auth_release;

GRANT EXECUTE ON FUNCTION public.auth_connection_release(uuid) TO chat_auth;
GRANT EXECUTE ON FUNCTION public.auth_pin_scd() TO chat_auth_release;
