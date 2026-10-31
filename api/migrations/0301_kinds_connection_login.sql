SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_connection_login(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    connection_id uuid := (args ->> 'connection_id')::uuid;
    v bigint := (args ->> 'login_v')::bigint;
    stamp timestamptz := (args ->> 'at')::timestamptz;
    installed boolean;
BEGIN
    UPDATE public.soundcloud_connections
    SET soundcloud_user_id = args ->> 'soundcloud_user_id',
        username = coalesce(nullif(args ->> 'username', ''), username),
        oauth_app_id = (args ->> 'oauth_app_id')::uuid,
        access_token = args ->> 'access_token',
        refresh_token = args ->> 'refresh_token',
        expires_at = (args ->> 'expires_at')::timestamptz,
        scope = args ->> 'scope',
        token_v = greatest(token_v, v),
        login_v = v,
        refresh_generation = refresh_generation + 1,
        refresh_failure_count = 0,
        refresh_rejection_count = 0,
        first_refresh_rejection_at = NULL,
        refresh_lease_id = NULL,
        refresh_lease_expires_at = NULL,
        last_refresh_success_at = stamp,
        last_refresh_error_kind = NULL,
        last_refresh_error = NULL,
        retry_at = NULL,
        updated_at = stamp
    WHERE id = connection_id AND login_v < v;
    installed := FOUND;
    IF NOT installed THEN
        UPDATE public.soundcloud_connections
        SET updated_at = greatest(updated_at, stamp)
        WHERE id = connection_id;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
        END IF;
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'soundcloud_connections', 'pk', jsonb_build_object('id', connection_id))),
        'output', installed
    );
END
$f$;
