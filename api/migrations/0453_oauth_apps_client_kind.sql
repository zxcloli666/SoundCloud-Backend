SET LOCAL lock_timeout = '5s';

ALTER TABLE public.oauth_apps
    ADD COLUMN client_kind text NOT NULL DEFAULT 'default',
    ADD COLUMN app_id text,
    ADD COLUMN app_version text,
    ADD COLUMN user_agent text,
    ADD CONSTRAINT oauth_apps_client_kind_check
        CHECK (client_kind IN ('default', 'mobile')),
    ADD CONSTRAINT oauth_apps_mobile_profile_check
        CHECK (client_kind = 'default'
               OR (app_id IS NOT NULL AND app_version IS NOT NULL AND user_agent IS NOT NULL));

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
  AND app.client_kind = 'default'
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
      AND app.client_kind = 'default'
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
      AND app.client_kind = 'default'
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

