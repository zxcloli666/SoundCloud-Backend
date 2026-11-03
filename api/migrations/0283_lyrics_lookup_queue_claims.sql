SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.lyrics_lookup_queue_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public
AS $f$
DECLARE
    open_wake timestamptz := CASE
        WHEN NEW.wake_message_id IS NOT NULL AND NEW.wake_durable_at IS NULL THEN NEW.updated_at
    END;
BEGIN
    INSERT INTO public.lyrics_lookup_queue (track_id, priority, next_run_at, created_at, wake_at)
    VALUES (NEW.track_id, NEW.priority, NEW.next_run_at, NEW.created_at, open_wake)
    ON CONFLICT (track_id) DO NOTHING;
    IF FOUND THEN
        RETURN NULL;
    END IF;

    UPDATE public.lyrics_lookup_queue AS queue
    SET priority = NEW.priority,
        next_run_at = NEW.next_run_at,
        created_at = NEW.created_at,
        wake_at = open_wake,
        claim_job_id = CASE WHEN queue.claim_state_generation = NEW.generation THEN queue.claim_job_id END,
        claim_job_generation = CASE
            WHEN queue.claim_state_generation = NEW.generation THEN queue.claim_job_generation
        END,
        claim_job_lease_id = CASE
            WHEN queue.claim_state_generation = NEW.generation THEN queue.claim_job_lease_id
        END,
        claim_expires_at = CASE
            WHEN queue.claim_state_generation = NEW.generation THEN queue.claim_expires_at
        END,
        claim_state_generation = CASE
            WHEN queue.claim_state_generation = NEW.generation THEN queue.claim_state_generation
        END
    WHERE queue.track_id = NEW.track_id
      AND (
          (queue.priority, queue.next_run_at, queue.created_at, queue.wake_at)
              IS DISTINCT FROM (NEW.priority, NEW.next_run_at, NEW.created_at, open_wake)
          OR queue.claim_state_generation <> NEW.generation
      );
    RETURN NULL;
END
$f$;

CREATE OR REPLACE FUNCTION public.lyrics_lookup_track_state_refresh()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public
AS $f$
DECLARE
    effective_title text := btrim(NEW.title);
    effective_artist text := COALESCE(
        NULLIF(btrim(NEW.metadata_artist), ''),
        NULLIF(btrim(NEW.uploader_username), ''),
        ''
    );
    effective_genius_url text := NULLIF(btrim(NEW.genius_url), '');
    inputs_changed boolean;
    external_synced boolean;
    external_plain boolean;
BEGIN
    SELECT NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL,
           NULLIF(btrim(cache.plain_text), '') IS NOT NULL
    INTO external_synced, external_plain
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id = NEW.sc_track_id
      AND cache.source IN ('lrclib', 'musixmatch', 'genius', 'netease');

    IF external_synced THEN
        DELETE FROM lyrics_lookup_state
        WHERE track_id = NEW.id;
        RETURN NEW;
    END IF;

    inputs_changed := TG_OP = 'INSERT'
        OR OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
        OR btrim(OLD.title) IS DISTINCT FROM effective_title
        OR COALESCE(
            NULLIF(btrim(OLD.metadata_artist), ''),
            NULLIF(btrim(OLD.uploader_username), ''),
            ''
        ) IS DISTINCT FROM effective_artist
        OR OLD.duration_ms IS DISTINCT FROM NEW.duration_ms
        OR OLD.genius_song_id IS DISTINCT FROM NEW.genius_song_id
        OR NULLIF(btrim(OLD.genius_url), '') IS DISTINCT FROM effective_genius_url
        OR OLD.release_date IS DISTINCT FROM NEW.release_date
        OR OLD.sc_created_at IS DISTINCT FROM NEW.sc_created_at;

    IF external_plain THEN
        UPDATE lyrics_lookup_state AS state
        SET sc_track_id = NEW.sc_track_id,
            generation = CASE WHEN inputs_changed THEN state.generation + 1 ELSE state.generation END,
            priority = LEAST(state.priority, NEW.index_priority),
            input_title = effective_title,
            input_artist = effective_artist,
            input_duration_ms = NEW.duration_ms,
            input_genius_song_id = NEW.genius_song_id,
            input_genius_url = effective_genius_url,
            input_release_date = NEW.release_date,
            input_sc_created_at = NEW.sc_created_at,
            updated_at = now()
        WHERE state.track_id = NEW.id
          AND (
              inputs_changed
              OR (
                  state.sc_track_id,
                  state.priority,
                  state.input_title,
                  state.input_artist,
                  state.input_duration_ms,
                  state.input_genius_song_id,
                  state.input_genius_url,
                  state.input_release_date,
                  state.input_sc_created_at
              ) IS DISTINCT FROM (
                  NEW.sc_track_id,
                  LEAST(state.priority, NEW.index_priority),
                  effective_title,
                  effective_artist,
                  NEW.duration_ms,
                  NEW.genius_song_id,
                  effective_genius_url,
                  NEW.release_date,
                  NEW.sc_created_at
              )
          );
        RETURN NEW;
    END IF;

    INSERT INTO lyrics_lookup_state (
        track_id,
        sc_track_id,
        status,
        generation,
        priority,
        next_run_at,
        input_title,
        input_artist,
        input_duration_ms,
        input_genius_song_id,
        input_genius_url,
        input_release_date,
        input_sc_created_at,
        wake_message_id,
        wake_generation,
        created_at
    )
    VALUES (
        NEW.id,
        NEW.sc_track_id,
        'pending',
        1,
        NEW.index_priority,
        now(),
        effective_title,
        effective_artist,
        NEW.duration_ms,
        NEW.genius_song_id,
        effective_genius_url,
        NEW.release_date,
        NEW.sc_created_at,
        gen_random_uuid(),
        1,
        NEW.created_at
    )
    ON CONFLICT (track_id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        status = CASE WHEN inputs_changed THEN 'pending' ELSE lyrics_lookup_state.status END,
        generation = CASE
            WHEN inputs_changed THEN lyrics_lookup_state.generation + 1
            ELSE lyrics_lookup_state.generation
        END,
        priority = LEAST(lyrics_lookup_state.priority, EXCLUDED.priority),
        next_run_at = CASE
            WHEN inputs_changed THEN now()
            ELSE lyrics_lookup_state.next_run_at
        END,
        input_title = EXCLUDED.input_title,
        input_artist = EXCLUDED.input_artist,
        input_duration_ms = EXCLUDED.input_duration_ms,
        input_genius_song_id = EXCLUDED.input_genius_song_id,
        input_genius_url = EXCLUDED.input_genius_url,
        input_release_date = EXCLUDED.input_release_date,
        input_sc_created_at = EXCLUDED.input_sc_created_at,
        miss_streak = CASE WHEN inputs_changed THEN 0 ELSE lyrics_lookup_state.miss_streak END,
        failure_streak = CASE WHEN inputs_changed THEN 0 ELSE lyrics_lookup_state.failure_streak END,
        last_outcome = CASE WHEN inputs_changed THEN NULL ELSE lyrics_lookup_state.last_outcome END,
        last_error = CASE WHEN inputs_changed THEN NULL ELSE lyrics_lookup_state.last_error END,
        retry_after_at = CASE WHEN inputs_changed THEN NULL ELSE lyrics_lookup_state.retry_after_at END,
        wake_message_id = CASE
            WHEN inputs_changed THEN gen_random_uuid()
            ELSE lyrics_lookup_state.wake_message_id
        END,
        wake_generation = CASE
            WHEN inputs_changed THEN lyrics_lookup_state.generation + 1
            ELSE lyrics_lookup_state.wake_generation
        END,
        wake_durable_at = CASE
            WHEN inputs_changed THEN NULL
            ELSE lyrics_lookup_state.wake_durable_at
        END,
        updated_at = now()
    WHERE inputs_changed
        OR (
            lyrics_lookup_state.sc_track_id,
            lyrics_lookup_state.priority,
            lyrics_lookup_state.input_title,
            lyrics_lookup_state.input_artist,
            lyrics_lookup_state.input_duration_ms,
            lyrics_lookup_state.input_genius_song_id,
            lyrics_lookup_state.input_genius_url,
            lyrics_lookup_state.input_release_date,
            lyrics_lookup_state.input_sc_created_at
        ) IS DISTINCT FROM (
            EXCLUDED.sc_track_id,
            LEAST(lyrics_lookup_state.priority, EXCLUDED.priority),
            EXCLUDED.input_title,
            EXCLUDED.input_artist,
            EXCLUDED.input_duration_ms,
            EXCLUDED.input_genius_song_id,
            EXCLUDED.input_genius_url,
            EXCLUDED.input_release_date,
            EXCLUDED.input_sc_created_at
        );

    RETURN NEW;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_lyrics_wake(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    track text := args ->> 'sc_track_id';
    stamp timestamptz := (args ->> 'at')::timestamptz;
    state_id uuid;
    wake record;
BEGIN
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
