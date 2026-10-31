SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_track_storage_lost(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    lost uuid;
BEGIN
    UPDATE public.tracks AS t
    SET storage_state   = 'pending',
        storage_quality = NULL,
        s3_verified_at  = NULL,
        s3_missing_at   = stamp,
        updated_at      = stamp
    WHERE t.urn = args ->> 'urn'
      AND t.storage_state = 'ok'
    RETURNING t.id INTO lost;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', lost))),
        'output', true);
END
$f$;
