SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_track_too_long(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    marked uuid;
BEGIN
    UPDATE public.tracks AS track
    SET storage_state      = 'too_long',
        index_state        = 'too_long',
        indexed_at         = NULL,
        transcribe_state   = 'disabled',
        hq_upgrade_pending = false,
        updated_at         = stamp
    WHERE track.sc_track_id = args ->> 'sc_track_id'
      AND (
          track.storage_state <> 'too_long'
          OR track.index_state <> 'too_long'
          OR track.transcribe_state IS DISTINCT FROM 'disabled'
          OR track.hq_upgrade_pending
      )
    RETURNING track.id INTO marked;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', marked))),
        'output', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_track_release_held(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    released uuid;
    duration integer;
BEGIN
    UPDATE public.tracks AS track
    SET pipeline_held = false,
        updated_at    = stamp
    WHERE track.sc_track_id = args ->> 'sc_track_id'
      AND track.pipeline_held
      AND track.deleted_at IS NULL
    RETURNING track.id, track.duration_ms INTO released, duration;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL::integer);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', released))),
        'output', duration);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_track_s3_state(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    verdict record;
    changed uuid;
    entry jsonb;
    touched jsonb := '[]';
BEGIN
    FOR verdict IN
        SELECT DISTINCT v.sc_track_id, v.found
        FROM (
            SELECT ok.sc_track_id, true AS found
            FROM jsonb_array_elements_text(coalesce(args -> 'ok', '[]')) AS ok (sc_track_id)
            UNION ALL
            SELECT missing.sc_track_id, false
            FROM jsonb_array_elements_text(coalesce(args -> 'missing', '[]')) AS missing (sc_track_id)
        ) AS v
        ORDER BY v.sc_track_id, v.found DESC
    LOOP
        IF verdict.found THEN
            UPDATE public.tracks AS track
            SET storage_state  = 'ok',
                s3_verified_at = stamp,
                s3_missing_at  = NULL
            WHERE track.sc_track_id = verdict.sc_track_id
              AND track.storage_state <> 'too_long'
            RETURNING track.id INTO changed;
        ELSE
            UPDATE public.tracks AS track
            SET storage_state   = CASE
                                      WHEN track.storage_state IN ('ok', 'missing') THEN 'pending'
                                      ELSE track.storage_state
                END,
                storage_quality = CASE
                                      WHEN track.storage_state IN ('ok', 'missing') THEN NULL
                                      ELSE track.storage_quality
                END,
                s3_verified_at  = NULL,
                s3_missing_at   = stamp
            WHERE track.sc_track_id = verdict.sc_track_id
              AND track.storage_state <> 'too_long'
            RETURNING track.id INTO changed;
        END IF;
        IF FOUND THEN
            entry := jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', changed));
            IF NOT touched @> jsonb_build_array(entry) THEN
                touched := touched || jsonb_build_array(entry);
            END IF;
        END IF;
    END LOOP;
    RETURN jsonb_build_object('touched', touched, 'output', jsonb_array_length(touched));
END
$f$;
