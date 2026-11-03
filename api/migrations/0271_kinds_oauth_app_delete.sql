SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_oauth_app_delete(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    gone uuid;
BEGIN
    DELETE FROM public.oauth_apps AS app
    WHERE app.id = (args ->> 'app_id')::uuid
    RETURNING app.id INTO gone;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'oauth_apps', 'pk', jsonb_build_object('id', gone))),
        'output', true
    );
END
$f$;
