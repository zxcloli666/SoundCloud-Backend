CREATE OR REPLACE FUNCTION public.auth_session(p_session_id uuid)
RETURNS TABLE (soundcloud_connection_id uuid, soundcloud_user_id text, chat_bind bytea, created_at timestamp)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
#variable_conflict use_column
BEGIN
    RETURN QUERY
    SELECT session.soundcloud_connection_id,
           connection.soundcloud_user_id::text,
           session.chat_bind,
           session.created_at
    FROM public.sessions AS session
    LEFT JOIN public.soundcloud_connections AS connection
        ON connection.id = session.soundcloud_connection_id
    WHERE session.id = p_session_id;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_session(uuid) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_sessions_alive(p_session_ids uuid[])
RETURNS SETOF uuid
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
BEGIN
    IF cardinality(p_session_ids) > 1000 THEN
        RAISE EXCEPTION 'auth_sessions_alive takes at most 1000 ids, got %', cardinality(p_session_ids)
            USING ERRCODE = '54000';
    END IF;
    RETURN QUERY
    SELECT session.id
    FROM public.sessions AS session
    WHERE session.id = ANY (p_session_ids);
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_sessions_alive(uuid[]) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_session_username(p_session_id uuid)
RETURNS text
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
BEGIN
    RETURN (
        SELECT other.username
        FROM public.sessions AS session
        JOIN public.soundcloud_connections AS own
            ON own.id = session.soundcloud_connection_id
        JOIN public.soundcloud_connections AS other
            ON other.soundcloud_user_id = own.soundcloud_user_id
        WHERE session.id = p_session_id
          AND other.username IS NOT NULL
        ORDER BY other.updated_at DESC
        LIMIT 1
    );
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_session_username(uuid) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_session_end(p_session_id uuid)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
DECLARE
    v_connection uuid;
BEGIN
    DELETE FROM public.sessions AS session
    WHERE session.id = p_session_id
    RETURNING session.soundcloud_connection_id INTO v_connection;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    IF v_connection IS NOT NULL THEN
        PERFORM public.scd_wt('connection.drop_orphan', jsonb_build_object('connection_id', v_connection));
    END IF;
    RETURN true;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_session_end(uuid) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_login_create(
    p_state text,
    p_code_verifier text,
    p_target_session_id uuid,
    p_chat_bind bytea
)
RETURNS TABLE (login_request_id uuid, client_id text, redirect_uri text)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
#variable_conflict use_column
DECLARE
    v_id uuid := gen_random_uuid();
    v_app uuid;
    v_client_id text;
    v_redirect_uri text;
    v_target uuid;
BEGIN
    IF p_chat_bind IS NULL THEN
        RAISE EXCEPTION 'auth_login_create needs a chat bind' USING ERRCODE = '22004';
    END IF;
    IF length(p_state) < 32 OR length(p_code_verifier) < 43 THEN
        RAISE EXCEPTION 'auth_login_create needs a full state and code verifier' USING ERRCODE = '22023';
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
    VALUES (v_id, p_state, p_code_verifier, v_app::text, v_target, 'pending',
            timezone('UTC', now()) + interval '15 minutes', p_chat_bind);

    RETURN QUERY SELECT v_id, v_client_id, v_redirect_uri;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_login_create(text, text, uuid, bytea) FROM PUBLIC;

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
    request public.login_requests%ROWTYPE;
BEGIN
    SELECT * INTO request
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

CREATE OR REPLACE FUNCTION public.auth_link_create(
    p_claim_token text,
    p_mode text,
    p_source_session_id uuid,
    p_chat_bind bytea
)
RETURNS TABLE (link_request_id uuid, expires_at timestamp)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
#variable_conflict use_column
DECLARE
    v_id uuid := gen_random_uuid();
    v_expires_at timestamp := timezone('UTC', now()) + interval '5 minutes';
    v_bind bytea := p_chat_bind;
    source public.sessions%ROWTYPE;
BEGIN
    IF p_mode IS NULL OR p_mode NOT IN ('pull', 'push') OR (p_mode = 'push') <> (p_source_session_id IS NOT NULL) THEN
        RAISE EXCEPTION 'auth_link_create got mode % with source %', p_mode, p_source_session_id IS NOT NULL
            USING ERRCODE = '22023';
    END IF;
    IF length(p_claim_token) < 32 THEN
        RAISE EXCEPTION 'auth_link_create needs a full claim token' USING ERRCODE = '22023';
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
    VALUES (v_id, p_claim_token, p_mode, p_source_session_id, NULL, 'pending', v_expires_at, v_bind);

    RETURN QUERY SELECT v_id, v_expires_at;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_link_create(text, text, uuid, bytea) FROM PUBLIC;

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
AS $f$
#variable_conflict use_column
DECLARE
    link public.link_requests%ROWTYPE;
    source public.sessions%ROWTYPE;
    v_source uuid;
    v_bind bytea;
    v_target uuid := gen_random_uuid();
BEGIN
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

CREATE OR REPLACE FUNCTION public.auth_link_status(p_link_request_id uuid)
RETURNS TABLE (status text, mode text, target_session_id uuid, error text, chat_bind bytea)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $f$
#variable_conflict use_column
DECLARE
    link public.link_requests%ROWTYPE;
BEGIN
    SELECT * INTO link
    FROM public.link_requests AS request
    WHERE request.id = p_link_request_id;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    IF link.status = 'pending' AND link.expires_at < timezone('UTC', now()) THEN
        RETURN QUERY SELECT 'expired'::text, link.mode::text, NULL::uuid, 'Expired'::text, NULL::bytea;
        RETURN;
    END IF;
    RETURN QUERY SELECT link.status::text,
                        link.mode::text,
                        CASE WHEN link.mode = 'pull' AND link.status = 'claimed' THEN link.target_session_id END,
                        link.error,
                        link.chat_bind;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_link_status(uuid) FROM PUBLIC;
