SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_lyrics_wake(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $f$
DECLARE
    track text := args ->> 'sc_track_id';
    stamp timestamptz := (args ->> 'at')::timestamptz;
    recheck_days integer := COALESCE((args ->> 'recheck_days')::integer, 7);
    state_id uuid;
    wake record;
BEGIN
    PERFORM 1 FROM public.tracks AS t WHERE t.sc_track_id = track FOR KEY SHARE;

    INSERT INTO public.lyrics_lookup_state (
        track_id, sc_track_id, priority, next_run_at, input_title, input_artist, input_duration_ms,
        input_genius_song_id, input_genius_url, input_release_date, input_sc_created_at,
        wake_message_id, wake_generation, created_at, updated_at
    )
    SELECT t.id, t.sc_track_id, 0, stamp, btrim(t.title),
           COALESCE(NULLIF(btrim(t.metadata_artist), ''), NULLIF(btrim(t.uploader_username), ''), ''),
           t.duration_ms, t.genius_song_id, NULLIF(btrim(t.genius_url), ''), t.release_date, t.sc_created_at,
           public.scd_uuid('lyrics.wake', t.sc_track_id, '1'), 1, t.created_at, stamp
    FROM public.tracks AS t
    WHERE t.sc_track_id = track
      AND NOT EXISTS (
          SELECT 1 FROM public.lyrics_cache AS cache
          WHERE cache.sc_track_id = t.sc_track_id
            AND (NULLIF(btrim(cache.plain_text), '') IS NOT NULL OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL)
            AND NOT (
                cache.source = 'lrclib'
                AND cache.synced_lrc IS NULL
                AND cache.updated_at < stamp - recheck_days * interval '1 day'
            )
      )
    ON CONFLICT (track_id) DO NOTHING;

    UPDATE public.lyrics_lookup_state AS state
    SET status = 'pending',
        generation = state.generation + 1,
        next_run_at = stamp,
        last_outcome = NULL,
        last_error = NULL,
        retry_after_at = NULL,
        wake_message_id = public.scd_uuid('lyrics.wake', state.sc_track_id, (state.generation + 1)::text),
        wake_generation = state.generation + 1,
        wake_durable_at = NULL
    WHERE state.sc_track_id = track
      AND state.status IN ('retry', 'not_found')
      AND state.next_run_at <= stamp;

    UPDATE public.lyrics_lookup_state AS state
    SET wake_message_id = public.scd_uuid('lyrics.wake', state.sc_track_id, state.generation::text),
        wake_generation = state.generation,
        wake_durable_at = NULL
    WHERE state.sc_track_id = track
      AND state.status = 'pending'
      AND state.wake_message_id IS NULL;

    UPDATE public.lyrics_lookup_state AS state
    SET priority = 0,
        updated_at = stamp
    WHERE state.sc_track_id = track
    RETURNING state.track_id, state.status, state.next_run_at, state.failure_streak,
              (state.wake_message_id IS NOT NULL AND state.wake_generation = state.generation
               AND state.wake_durable_at IS NULL) AS wake_open
    INTO wake;

    IF wake.track_id IS NULL THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL);
    END IF;
    state_id := wake.track_id;
    IF wake.wake_open AND public.scd_role() = 'core' THEN
        PERFORM pg_notify('lyrics_wake', track);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'lyrics_lookup_state', 'pk', jsonb_build_object('track_id', state_id))),
        'output', jsonb_build_object(
            'status', wake.status, 'next_run_at', wake.next_run_at,
            'failure_streak', wake.failure_streak));
END
$f$;
