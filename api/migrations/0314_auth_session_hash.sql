SET LOCAL lock_timeout = '5s';

DO $do$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_index AS index
        WHERE index.indexrelid = to_regclass('public.sessions_id_sha256_idx')
          AND index.indisvalid
    ) THEN
        RAISE EXCEPTION 'public.sessions_id_sha256_idx is missing or invalid'
            USING HINT = 'DROP INDEX CONCURRENTLY IF EXISTS public.sessions_id_sha256_idx, then rerun the core migrations';
    END IF;
END
$do$;

DO $do$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'chat'
          AND table_name = 'devices'
          AND column_name = 'session_id'
    ) THEN
        ALTER TABLE chat.devices
            ALTER COLUMN session_id TYPE bytea USING sha256(uuid_send(session_id));
        ALTER TABLE chat.devices RENAME COLUMN session_id TO session_hash;
    END IF;
END
$do$;

CREATE SCHEMA IF NOT EXISTS auth_caps;

CREATE TABLE IF NOT EXISTS auth_caps.starts (
    kind text NOT NULL,
    at timestamptz NOT NULL
);

CREATE INDEX IF NOT EXISTS starts_kind_at ON auth_caps.starts (kind, at);

CREATE OR REPLACE FUNCTION public.auth_pin_scd()
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp
AS $f$
DECLARE
    v_name text;
    v_value text;
BEGIN
    FOREACH v_name IN ARRAY ARRAY['scd.mode', 'scd.node', 'scd.role', 'scd.merging', 'scd.cmd'] LOOP
        v_value := '';
        IF v_name IN ('scd.mode', 'scd.node', 'scd.role') THEN
            SELECT substr(config, length(v_name) + 2) INTO v_value
            FROM pg_catalog.pg_db_role_setting AS setting,
                 unnest(setting.setconfig) AS config
            WHERE setting.setrole = 0
              AND setting.setdatabase = (
                      SELECT database.oid
                      FROM pg_catalog.pg_database AS database
                      WHERE database.datname = current_database()
                  )
              AND config LIKE v_name || '=%';
        END IF;
        PERFORM pg_catalog.set_config(v_name, coalesce(v_value, ''), true);
    END LOOP;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_pin_scd() FROM PUBLIC;

DROP FUNCTION IF EXISTS public.auth_sessions_alive(uuid[]);
DROP FUNCTION IF EXISTS public.auth_session_end(uuid);
DROP FUNCTION IF EXISTS public.auth_login_create(text, text, uuid, bytea);
DROP FUNCTION IF EXISTS public.auth_link_create(text, text, uuid, bytea);

CREATE OR REPLACE FUNCTION public.auth_secret(p_bytes int)
RETURNS text
LANGUAGE sql
VOLATILE
SET search_path = pg_catalog, public, pg_temp
AS $f$
SELECT rtrim(translate(encode(substr(
    uuid_send(gen_random_uuid()) || uuid_send(gen_random_uuid()) || uuid_send(gen_random_uuid()),
    1, p_bytes), 'base64'), '+/', '-_'), '=')
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_secret(int) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_uuid_v7()
RETURNS uuid
LANGUAGE sql
VOLATILE
SET search_path = pg_catalog, public, pg_temp
AS $f$
SELECT encode(set_bit(set_bit(overlay(uuid_send(gen_random_uuid())
                                      PLACING substring(int8send(floor(extract(EPOCH FROM clock_timestamp()) * 1000)::bigint) FROM 3)
                                      FROM 1 FOR 6),
                              52, 1),
                      53, 1),
              'hex')::uuid
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_uuid_v7() FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_start_window(p_kind text, p_limit int)
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp
AS $f$
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'auth starts run in read committed, not %', current_setting('transaction_isolation')
            USING ERRCODE = '25000';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('auth-start-' || p_kind, 0));
    DELETE FROM auth_caps.starts AS start
    WHERE start.kind = p_kind
      AND start.at <= clock_timestamp() - interval '1 minute';
    IF (SELECT count(*) FROM auth_caps.starts AS start WHERE start.kind = p_kind) >= p_limit THEN
        RAISE EXCEPTION '% % starts in the last minute', p_limit, p_kind USING ERRCODE = 'AU429';
    END IF;
    INSERT INTO auth_caps.starts (kind, at) VALUES (p_kind, clock_timestamp());
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_start_window(text, int) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_sessions_alive(p_session_hashes bytea[])
RETURNS SETOF bytea
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
BEGIN
    IF cardinality(p_session_hashes) > 1000 THEN
        RAISE EXCEPTION 'auth_sessions_alive takes at most 1000 hashes, got %', cardinality(p_session_hashes)
            USING ERRCODE = '54000';
    END IF;
    RETURN QUERY
    SELECT sha256(uuid_send(session.id))
    FROM public.sessions AS session
    WHERE sha256(uuid_send(session.id)) = ANY (p_session_hashes);
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_sessions_alive(bytea[]) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_session_logged_out(p_session_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
SELECT EXISTS (
    SELECT 1
    FROM public.repl_tombstones AS tombstone
    WHERE tombstone.tbl = 'sessions'
      AND tombstone.pk = jsonb_build_object('id', p_session_id)
)
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_session_logged_out(uuid) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_connection_release(p_connection_id uuid)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
SET "TimeZone" = 'UTC'
SET scd.mode = ''
SET scd.node = ''
SET scd.role = ''
SET scd.merging = ''
SET scd.cmd = ''
SET scd.writer = ''
SET scd.guard = ''
AS $f$
BEGIN
    PERFORM public.auth_pin_scd();
    PERFORM public.scd_wt('connection.drop_orphan', jsonb_build_object('connection_id', p_connection_id));
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_connection_release(uuid) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_session_end(p_caller_session_id uuid, p_session_hash bytea)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
SET "TimeZone" = 'UTC'
SET scd.mode = ''
SET scd.node = ''
SET scd.role = ''
SET scd.merging = ''
SET scd.cmd = ''
SET scd.writer = ''
SET scd.guard = ''
AS $f$
DECLARE
    v_session uuid;
    v_connection uuid;
BEGIN
    PERFORM public.auth_pin_scd();
    DELETE FROM public.sessions AS session
    WHERE sha256(uuid_send(session.id)) = p_session_hash
      AND (
          session.id = p_caller_session_id
          OR EXISTS (
              SELECT 1
              FROM public.sessions AS caller
              JOIN public.soundcloud_connections AS own
                  ON own.id = caller.soundcloud_connection_id
              JOIN public.soundcloud_connections AS target
                  ON target.soundcloud_user_id = own.soundcloud_user_id
              WHERE caller.id = p_caller_session_id
                AND target.id = session.soundcloud_connection_id
          )
      )
    RETURNING session.id, session.soundcloud_connection_id INTO v_session, v_connection;
    IF v_connection IS NOT NULL THEN
        PERFORM public.auth_connection_release(v_connection);
    END IF;
    RETURN v_session IS NOT NULL;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_session_end(uuid, bytea) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_login_apps()
RETURNS TABLE (oauth_app_id uuid, client_id text)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
SELECT app.id, app.client_id
FROM public.oauth_apps AS app
WHERE app.active
ORDER BY app.created_at, app.id
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_login_apps() FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_login_create(
    p_target_session_id uuid,
    p_chat_bind bytea,
    p_unhealthy_app_ids uuid[]
)
RETURNS TABLE (
    login_request_id uuid,
    state text,
    code_challenge text,
    client_id text,
    redirect_uri text
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
SET "TimeZone" = 'UTC'
SET scd.mode = ''
SET scd.node = ''
SET scd.role = ''
SET scd.merging = ''
SET scd.cmd = ''
SET scd.writer = ''
SET scd.guard = ''
AS $f$
#variable_conflict use_column
DECLARE
    v_id uuid := public.auth_uuid_v7();
    v_state text := encode(uuid_send(gen_random_uuid()), 'hex');
    v_code_verifier text := public.auth_secret(32);
    v_app uuid;
    v_client_id text;
    v_redirect_uri text;
    v_target uuid;
BEGIN
    IF p_chat_bind IS NULL THEN
        RAISE EXCEPTION 'auth_login_create needs a chat bind' USING ERRCODE = '22004';
    END IF;
    IF octet_length(p_chat_bind) > 512 THEN
        RAISE EXCEPTION 'a chat bind holds at most 512 bytes' USING ERRCODE = '22001';
    END IF;
    IF cardinality(p_unhealthy_app_ids) > 100 THEN
        RAISE EXCEPTION 'auth_login_create takes at most 100 unhealthy apps, got %', cardinality(p_unhealthy_app_ids)
            USING ERRCODE = '54000';
    END IF;
    PERFORM public.auth_pin_scd();
    IF NOT has_table_privilege(session_user, 'public.login_requests', 'INSERT') THEN
        PERFORM public.auth_start_window('login', 3000);
    END IF;

    SELECT app.id INTO v_app
    FROM public.oauth_apps AS app
    LEFT JOIN public.oauth_app_request_cooldowns AS cooldown
        ON cooldown.oauth_app_id = app.id
       AND cooldown.retry_at > now()
    WHERE app.active
    ORDER BY EXISTS (
                 SELECT 1
                 FROM public.sync_queue_action_pauses AS pause
                 WHERE pause.oauth_app_id = app.id
                   AND pause.reason = 'subscription_required'
             ),
             cooldown.retry_at IS NOT NULL,
             greatest(1, ceil(extract(EPOCH FROM cooldown.retry_at - now()))) NULLS FIRST,
             coalesce(app.id = ANY (p_unhealthy_app_ids), false),
             app.last_used_at ASC NULLS FIRST,
             app.created_at ASC
    LIMIT 1
    FOR UPDATE OF app SKIP LOCKED;

    IF v_app IS NULL THEN
        SELECT app.id INTO v_app
        FROM public.oauth_apps AS app
        WHERE app.active
        ORDER BY app.last_used_at ASC NULLS FIRST, app.created_at ASC
        LIMIT 1
        FOR UPDATE OF app;
    END IF;
    IF v_app IS NULL THEN
        RETURN;
    END IF;

    UPDATE public.oauth_apps AS app
    SET last_used_at = now(),
        updated_at = now()
    WHERE app.id = v_app
    RETURNING app.client_id, app.redirect_uri INTO v_client_id, v_redirect_uri;

    SELECT session.id INTO v_target
    FROM public.sessions AS session
    WHERE session.id = p_target_session_id;

    INSERT INTO public.login_requests
        (id, state, code_verifier, oauth_app_id, target_session_id, status, expires_at, chat_bind)
    VALUES (v_id, v_state, v_code_verifier, v_app::text, v_target, 'pending',
            timezone('UTC', now()) + interval '15 minutes', p_chat_bind);

    RETURN QUERY SELECT v_id,
                        v_state,
                        rtrim(translate(encode(sha256(convert_to(v_code_verifier, 'UTF8')), 'base64'), '+/', '-_'), '='),
                        v_client_id,
                        v_redirect_uri;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_login_create(uuid, bytea, uuid[]) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_login_status(p_login_request_id uuid)
RETURNS TABLE (
    status text,
    step text,
    username text,
    result_session_id uuid,
    error text,
    redirect_url text,
    profile_ok boolean,
    chat_bind bytea
)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
#variable_conflict use_column
DECLARE
    request record;
BEGIN
    SELECT login.status,
           login.step,
           login.username,
           login.result_session_id,
           login.error,
           login.redirect_url,
           login.profile_ok,
           login.chat_bind,
           login.expires_at
    INTO request
    FROM public.login_requests AS login
    WHERE login.id = p_login_request_id;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    IF request.status IN ('pending', 'processing') AND request.expires_at < timezone('UTC', now()) THEN
        RETURN QUERY SELECT 'expired'::text, NULL::text, NULL::text, NULL::uuid, 'Login request expired'::text,
                            NULL::text, NULL::boolean, NULL::bytea;
        RETURN;
    END IF;
    RETURN QUERY SELECT request.status::text,
                        request.step::text,
                        request.username,
                        CASE WHEN request.status = 'completed' THEN request.result_session_id END,
                        request.error,
                        request.redirect_url,
                        request.profile_ok,
                        request.chat_bind;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_login_status(uuid) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_link_create(p_mode text, p_source_session_id uuid, p_chat_bind bytea)
RETURNS TABLE (link_request_id uuid, claim_token text, expires_at timestamp)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
SET "TimeZone" = 'UTC'
SET scd.mode = ''
SET scd.node = ''
SET scd.role = ''
SET scd.merging = ''
SET scd.cmd = ''
SET scd.writer = ''
SET scd.guard = ''
AS $f$
#variable_conflict use_column
DECLARE
    v_id uuid := public.auth_uuid_v7();
    v_claim_token text := public.auth_secret(24);
    v_expires_at timestamp := timezone('UTC', now()) + interval '5 minutes';
    v_bind bytea := p_chat_bind;
    source public.sessions%ROWTYPE;
BEGIN
    IF p_mode IS NULL OR p_mode NOT IN ('pull', 'push') OR (p_mode = 'push') <> (p_source_session_id IS NOT NULL) THEN
        RAISE EXCEPTION 'auth_link_create got mode % with source %', p_mode, p_source_session_id IS NOT NULL
            USING ERRCODE = '22023';
    END IF;
    IF octet_length(p_chat_bind) > 512 THEN
        RAISE EXCEPTION 'a chat bind holds at most 512 bytes' USING ERRCODE = '22001';
    END IF;
    PERFORM public.auth_pin_scd();
    IF NOT has_table_privilege(session_user, 'public.link_requests', 'INSERT') THEN
        PERFORM public.auth_start_window('link', 3000);
    END IF;
    IF p_mode = 'push' THEN
        SELECT * INTO source
        FROM public.sessions AS session
        WHERE session.id = p_source_session_id;
        IF NOT FOUND OR source.soundcloud_connection_id IS NULL THEN
            RETURN;
        END IF;
        IF v_bind IS DISTINCT FROM source.chat_bind THEN
            v_bind := NULL;
        END IF;
    END IF;

    INSERT INTO public.link_requests
        (id, claim_token, mode, source_session_id, target_session_id, status, expires_at, chat_bind)
    VALUES (v_id, v_claim_token, p_mode, p_source_session_id, NULL, 'pending', v_expires_at, v_bind);

    RETURN QUERY SELECT v_id, v_claim_token, v_expires_at;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_link_create(text, uuid, bytea) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_link_claim(
    p_claim_token text,
    p_caller_session_id uuid,
    p_caller_bind bytea,
    p_proven_bind bytea
)
RETURNS TABLE (outcome text, session_id uuid, mode text)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
SET "TimeZone" = 'UTC'
SET scd.mode = ''
SET scd.node = ''
SET scd.role = ''
SET scd.merging = ''
SET scd.cmd = ''
SET scd.writer = ''
SET scd.guard = ''
AS $f$
#variable_conflict use_column
DECLARE
    link public.link_requests%ROWTYPE;
    source public.sessions%ROWTYPE;
    v_source uuid;
    v_bind bytea;
    v_target uuid := public.auth_uuid_v7();
BEGIN
    IF octet_length(p_caller_bind) > 512 OR octet_length(p_proven_bind) > 512 THEN
        RAISE EXCEPTION 'a chat bind holds at most 512 bytes' USING ERRCODE = '22001';
    END IF;
    PERFORM public.auth_pin_scd();
    SELECT * INTO link
    FROM public.link_requests AS request
    WHERE request.claim_token = p_claim_token
      AND request.status = 'pending'
    FOR UPDATE;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'unknown'::text, NULL::uuid, NULL::text;
        RETURN;
    END IF;
    IF link.expires_at < timezone('UTC', now()) THEN
        UPDATE public.link_requests AS request
        SET status = 'failed',
            error = 'Expired'
        WHERE request.id = link.id;
        RETURN QUERY SELECT 'expired'::text, NULL::uuid, link.mode::text;
        RETURN;
    END IF;

    v_source := CASE link.mode
        WHEN 'pull' THEN p_caller_session_id
        WHEN 'push' THEN link.source_session_id
    END;
    IF v_source IS NULL THEN
        RETURN QUERY SELECT CASE link.mode
                                WHEN 'pull' THEN 'pull_needs_source'
                                WHEN 'push' THEN 'push_without_source'
                                ELSE 'unknown_mode'
                            END,
                            NULL::uuid,
                            link.mode::text;
        RETURN;
    END IF;

    SELECT * INTO source
    FROM public.sessions AS session
    WHERE session.id = v_source
    FOR UPDATE;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'source_missing'::text, NULL::uuid, link.mode::text;
        RETURN;
    END IF;
    IF source.soundcloud_connection_id IS NULL THEN
        RETURN QUERY SELECT 'reauthorize'::text, NULL::uuid, link.mode::text;
        RETURN;
    END IF;

    v_bind := CASE
        WHEN link.mode = 'pull' AND p_proven_bind = source.chat_bind THEN link.chat_bind
        WHEN link.mode = 'push' AND link.chat_bind = source.chat_bind THEN p_caller_bind
    END;

    INSERT INTO public.sessions (id, soundcloud_connection_id, chat_bind)
    VALUES (v_target, source.soundcloud_connection_id, v_bind);

    UPDATE public.link_requests AS request
    SET source_session_id = v_source,
        target_session_id = v_target,
        status = 'claimed'
    WHERE request.id = link.id;

    RETURN QUERY SELECT 'claimed'::text, v_target, link.mode::text;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_link_claim(text, uuid, bytea, bytea) FROM PUBLIC;
