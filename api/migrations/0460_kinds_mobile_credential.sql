SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_mobile_credential_link(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $f$
DECLARE
    account text := args ->> 'soundcloud_user_id';
    v bigint := (args ->> 'link_v')::bigint;
    stamp timestamptz := (args ->> 'at')::timestamptz;
    installed boolean;
BEGIN
    IF args ->> 'access_token' IS NULL OR args ->> 'refresh_token' IS NULL THEN
        RAISE EXCEPTION 'mobile_credential.link of %: the token pair is missing or was redacted', account
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    UPDATE public.mobile_credentials
    SET oauth_app_id = (args ->> 'oauth_app_id')::uuid,
        device_id = (args ->> 'device_id')::uuid,
        access_token = args ->> 'access_token',
        refresh_token = args ->> 'refresh_token',
        expires_at = (args ->> 'expires_at')::timestamptz,
        scope = args ->> 'scope',
        linked_at = stamp,
        token_v = greatest(token_v, v),
        link_v = v,
        status = 'active',
        refresh_generation = refresh_generation + 1,
        refresh_failure_count = 0,
        refresh_lease_id = NULL,
        refresh_lease_expires_at = NULL,
        last_refresh_success_at = stamp,
        last_refresh_error_kind = NULL,
        last_refresh_error = NULL,
        retry_at = NULL,
        updated_at = stamp
    WHERE soundcloud_user_id = account AND link_v < v;
    installed := FOUND;
    IF NOT installed THEN
        UPDATE public.mobile_credentials
        SET updated_at = greatest(updated_at, stamp)
        WHERE soundcloud_user_id = account;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
        END IF;
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'mobile_credentials', 'pk', jsonb_build_object('soundcloud_user_id', account))),
        'output', installed
    );
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_mobile_credential_drop(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $f$
DECLARE
    account text := args ->> 'soundcloud_user_id';
    seen bigint;
BEGIN
    SELECT greatest(credential.consent_v, credential.plan_v, credential.token_v) INTO seen
    FROM public.mobile_credentials AS credential
    WHERE credential.soundcloud_user_id = account;
    IF NOT FOUND OR NOT public.scd_purge_intent('mobile_credentials', jsonb_build_object('soundcloud_user_id', account), seen) THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'mobile_credentials', 'pk', jsonb_build_object('soundcloud_user_id', account))),
        'output', true
    );
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_connection_drop_orphan(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $f$
DECLARE
    connection_id uuid := (args ->> 'connection_id')::uuid;
    seen bigint;
    account text;
    mobile_seen bigint;
    touched jsonb;
BEGIN
    SELECT connection.token_v, connection.soundcloud_user_id INTO seen, account
    FROM public.soundcloud_connections AS connection
    WHERE connection.id = connection_id
      AND NOT EXISTS (
          SELECT 1
          FROM public.sessions AS session
          WHERE session.soundcloud_connection_id = connection.id
      );
    IF NOT FOUND OR NOT public.scd_purge_intent('soundcloud_connections', jsonb_build_object('id', connection_id), seen) THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    touched := jsonb_build_array(jsonb_build_object('tbl', 'soundcloud_connections', 'pk', jsonb_build_object('id', connection_id)));
    IF NOT EXISTS (
        SELECT 1
        FROM public.soundcloud_connections AS other
        WHERE other.soundcloud_user_id = account
    ) THEN
        SELECT greatest(credential.consent_v, credential.plan_v, credential.token_v) INTO mobile_seen
        FROM public.mobile_credentials AS credential
        WHERE credential.soundcloud_user_id = account;
        IF FOUND AND public.scd_purge_intent('mobile_credentials', jsonb_build_object('soundcloud_user_id', account), mobile_seen) THEN
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'mobile_credentials', 'pk', jsonb_build_object('soundcloud_user_id', account)));
        END IF;
    END IF;
    RETURN jsonb_build_object('touched', touched, 'output', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_redact_args(kind text, args jsonb)
RETURNS jsonb
LANGUAGE sql
IMMUTABLE
SET search_path = public, pg_catalog, pg_temp
AS $f$
SELECT CASE
    WHEN kind IN ('connection.login', 'mobile_credential.link') AND args ?| ARRAY['access_token', 'refresh_token']
        THEN (args - ARRAY['access_token', 'refresh_token']) || jsonb_build_object('redacted', true)
    ELSE args
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_mobile_link_replayable(args jsonb)
RETURNS boolean
LANGUAGE plpgsql
STABLE
SET search_path = public, pg_catalog, pg_temp
AS $f$
BEGIN
    RETURN EXISTS (
        SELECT 1 FROM public.mobile_credentials c
        WHERE c.soundcloud_user_id = args ->> 'soundcloud_user_id' AND c.link_v < (args ->> 'link_v')::bigint
    );
EXCEPTION WHEN invalid_text_representation OR numeric_value_out_of_range THEN
    RETURN false;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_dead_redact(batch integer DEFAULT 1000)
RETURNS bigint
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
SET lock_timeout = '2s'
AS $f$
DECLARE
    n bigint;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd retention runs on core only' USING ERRCODE = 'feature_not_supported';
    END IF;
    WITH r AS (
        UPDATE public.cmd_dead t SET args = public.scd_redact_args(t.kind, t.args)
        WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_dead x
            WHERE (x.kind = 'connection.login' AND x.args ?| ARRAY['access_token', 'refresh_token']
                   AND NOT public.scd_login_replayable(x.args))
               OR (x.kind = 'mobile_credential.link' AND x.args ?| ARRAY['access_token', 'refresh_token']
                   AND NOT public.scd_mobile_link_replayable(x.args))
            ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO n FROM r;
    RETURN n;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_node_retention_batch(
    tbl text,
    keep interval DEFAULT interval '2 days',
    batch integer DEFAULT 1000,
    lag_bytes bigint DEFAULT 67108864,
    stale interval DEFAULT interval '3 minutes',
    OUT deleted bigint,
    OUT redacted bigint,
    OUT held text
)
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
SET lock_timeout = '2s'
AS $f$
DECLARE
    done_keep interval := least(greatest(keep, interval '1 day'), public.scd_dedup_window() - interval '1 day');
    secret_cutoff timestamptz := now() - interval '1 hour';
    stuck bigint;
BEGIN
    deleted := 0;
    redacted := 0;
    IF tbl NOT IN ('cmd_outbox', 'oauth_app_tokens_local', 'repl_purges') THEN
        RAISE EXCEPTION 'scd node retention: unknown table %', tbl USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF tbl = 'repl_purges' AND (public.scd_mode() <> 'split' OR public.scd_role() <> 'api') THEN
        RETURN;
    END IF;
    IF NOT pg_try_advisory_xact_lock(hashtextextended('scd.retention.node', 0)) THEN
        RETURN;
    END IF;
    IF tbl = 'oauth_app_tokens_local' THEN
        WITH d AS (
            DELETE FROM public.oauth_app_tokens_local t WHERE t.ctid IN (
                SELECT x.ctid FROM public.oauth_app_tokens_local x
                WHERE x.expires_at < secret_cutoff ORDER BY x.expires_at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
        RETURN;
    END IF;
    SELECT string_agg(g.peer || ': ' || g.reason, '; ' ORDER BY g.peer, g.reason) INTO held FROM public.scd_node_retention_guard(lag_bytes, stale) g;
    IF held IS NOT NULL THEN
        RETURN;
    END IF;
    IF tbl = 'repl_purges' THEN
        WITH d AS (
            DELETE FROM public.repl_purges t WHERE t.ctid IN (
                SELECT x.ctid FROM public.repl_purges x
                WHERE x.at < now() - greatest(keep, public.scd_dedup_window()) ORDER BY x.at LIMIT batch FOR UPDATE SKIP LOCKED
            ) RETURNING 1
        ) SELECT count(*) INTO deleted FROM d;
        RETURN;
    END IF;
    WITH r AS (
        UPDATE public.cmd_outbox t SET args = public.scd_redact_args(t.kind, t.args)
        WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_outbox x
            WHERE x.kind = 'connection.login' AND x.args ?| ARRAY['access_token', 'refresh_token']
              AND (x.done_at < secret_cutoff OR (x.done_at IS NULL AND x.created_at < now() - public.scd_dedup_window()))
            ORDER BY x.created_at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO redacted FROM r;
    WITH r AS (
        UPDATE public.cmd_outbox t SET args = public.scd_redact_args(t.kind, t.args)
        WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_outbox x
            WHERE x.kind = 'mobile_credential.link' AND x.args ?| ARRAY['access_token', 'refresh_token']
              AND (x.done_at < secret_cutoff OR (x.done_at IS NULL AND x.created_at < now() - public.scd_dedup_window()))
            ORDER BY x.created_at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT redacted + count(*) INTO redacted FROM r;
    WITH d AS (
        DELETE FROM public.cmd_outbox t WHERE t.ctid IN (
            SELECT x.ctid FROM public.cmd_outbox x
            WHERE x.created_at < now() - done_keep AND x.done_at < now() - done_keep
            ORDER BY x.created_at LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO deleted FROM d;
    SELECT count(*) INTO stuck FROM (
        SELECT 1 FROM public.cmd_outbox x WHERE x.created_at < now() - done_keep AND x.done_at IS NULL LIMIT 100000
    ) s;
    IF stuck > 0 THEN
        held := format('%s commands wait for core longer than %s', stuck, done_keep);
    END IF;
END
$f$;

GRANT SELECT (soundcloud_user_id) ON public.soundcloud_connections TO chat_auth_release;
GRANT SELECT (soundcloud_user_id, token_v, plan_v, consent_v), UPDATE (updated_at), DELETE ON public.mobile_credentials TO chat_auth_release;
