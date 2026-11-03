CREATE OR REPLACE FUNCTION lyrics_lookup_track_state_refresh()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    effective_title text;
    effective_artist text;
    effective_genius_url text;
    inputs_changed boolean;
    has_external_lyrics boolean;
BEGIN
    effective_title := btrim(NEW.title);
    effective_artist := COALESCE(
        NULLIF(btrim(NEW.metadata_artist), ''),
        NULLIF(btrim(NEW.uploader_username), ''),
        ''
    );
    effective_genius_url := NULLIF(btrim(NEW.genius_url), '');

    SELECT EXISTS (
        SELECT 1
        FROM lyrics_cache AS cache
        WHERE cache.sc_track_id = NEW.sc_track_id
          AND cache.source IN ('lrclib', 'musixmatch', 'genius', 'netease')
          AND (
              NULLIF(btrim(cache.plain_text), '') IS NOT NULL
              OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL
          )
    )
    INTO has_external_lyrics;

    IF has_external_lyrics THEN
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
        claim_job_id = CASE WHEN inputs_changed THEN NULL ELSE lyrics_lookup_state.claim_job_id END,
        claim_job_generation = CASE
            WHEN inputs_changed THEN NULL
            ELSE lyrics_lookup_state.claim_job_generation
        END,
        claim_job_lease_id = CASE
            WHEN inputs_changed THEN NULL
            ELSE lyrics_lookup_state.claim_job_lease_id
        END,
        claim_state_generation = CASE
            WHEN inputs_changed THEN NULL
            ELSE lyrics_lookup_state.claim_state_generation
        END,
        claim_expires_at = CASE
            WHEN inputs_changed THEN NULL
            ELSE lyrics_lookup_state.claim_expires_at
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
$$;
