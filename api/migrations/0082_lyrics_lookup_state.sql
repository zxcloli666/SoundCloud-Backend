ALTER TABLE lyrics_cache
    ADD COLUMN IF NOT EXISTS track_id uuid REFERENCES tracks(id) ON DELETE CASCADE,
    ADD COLUMN IF NOT EXISTS content_generation bigint NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS content_hash bytea,
    ADD COLUMN IF NOT EXISTS plain_source varchar(16),
    ADD COLUMN IF NOT EXISTS synced_source varchar(16),
    ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_text_present CHECK (
        NULLIF(btrim(plain_text), '') IS NOT NULL
        OR NULLIF(btrim(synced_lrc), '') IS NOT NULL
    ) NOT VALID;

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_text_size CHECK (
        octet_length(COALESCE(plain_text, '')) <= 819200
        AND octet_length(COALESCE(synced_lrc, '')) <= 819200
    ) NOT VALID;

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_source_valid CHECK (
        source IN ('lrclib', 'musixmatch', 'genius', 'netease', 'self_gen')
    ) NOT VALID;

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_plain_source_valid CHECK (
        plain_source IS NULL
        OR plain_source IN ('lrclib', 'musixmatch', 'genius', 'netease', 'self_gen')
    ) NOT VALID;

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_synced_source_valid CHECK (
        synced_source IS NULL
        OR synced_source IN ('lrclib', 'musixmatch', 'genius', 'netease', 'self_gen')
    ) NOT VALID;

CREATE INDEX IF NOT EXISTS lyrics_cache_track_id_idx
    ON lyrics_cache (track_id)
    WHERE track_id IS NOT NULL;

CREATE TABLE lyrics_lookup_state (
    track_id uuid PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    sc_track_id text NOT NULL UNIQUE,
    status varchar(16) NOT NULL DEFAULT 'pending',
    generation bigint NOT NULL DEFAULT 1,
    algorithm_version integer NOT NULL DEFAULT 1,
    source_set_version integer NOT NULL DEFAULT 1,
    priority smallint NOT NULL,
    next_run_at timestamptz NOT NULL DEFAULT now(),
    input_title text NOT NULL,
    input_artist text NOT NULL,
    input_duration_ms integer NOT NULL,
    input_genius_song_id bigint,
    input_genius_url text,
    input_release_date date,
    input_sc_created_at timestamptz,
    attempts bigint NOT NULL DEFAULT 0,
    miss_streak integer NOT NULL DEFAULT 0,
    failure_streak integer NOT NULL DEFAULT 0,
    last_attempt_at timestamptz,
    last_outcome varchar(32),
    last_error varchar(512),
    retry_after_at timestamptz,
    wake_message_id uuid,
    wake_generation bigint,
    wake_durable_at timestamptz,
    claim_job_id uuid,
    claim_job_generation bigint,
    claim_job_lease_id uuid,
    claim_state_generation bigint,
    claim_expires_at timestamptz,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT lyrics_lookup_state_status_valid CHECK (
        status IN ('pending', 'retry', 'not_found')
    ),
    CONSTRAINT lyrics_lookup_state_generation_valid CHECK (
        generation > 0
        AND algorithm_version > 0
        AND source_set_version > 0
        AND attempts >= 0
        AND miss_streak >= 0
        AND failure_streak >= 0
    ),
    CONSTRAINT lyrics_lookup_state_priority_valid CHECK (
        priority BETWEEN 0 AND 5
    ),
    CONSTRAINT lyrics_lookup_state_wake_valid CHECK (
        (wake_message_id IS NULL AND wake_generation IS NULL AND wake_durable_at IS NULL)
        OR (
            wake_message_id IS NOT NULL
            AND wake_generation IS NOT NULL
            AND wake_generation > 0
        )
    ),
    CONSTRAINT lyrics_lookup_state_claim_valid CHECK (
        (
            claim_job_id IS NULL
            AND claim_job_generation IS NULL
            AND claim_job_lease_id IS NULL
            AND claim_state_generation IS NULL
            AND claim_expires_at IS NULL
        )
        OR (
            claim_job_id IS NOT NULL
            AND claim_job_generation IS NOT NULL
            AND claim_job_generation > 0
            AND claim_job_lease_id IS NOT NULL
            AND claim_state_generation IS NOT NULL
            AND claim_state_generation > 0
            AND claim_expires_at IS NOT NULL
        )
    )
);

CREATE INDEX lyrics_lookup_due_priority_idx
    ON lyrics_lookup_state (priority, next_run_at, created_at, track_id)
    WHERE claim_job_id IS NULL;

CREATE INDEX lyrics_lookup_due_oldest_idx
    ON lyrics_lookup_state (next_run_at, created_at, track_id)
    WHERE claim_job_id IS NULL;

CREATE INDEX lyrics_lookup_expired_claim_idx
    ON lyrics_lookup_state (claim_expires_at, track_id)
    WHERE claim_job_id IS NOT NULL;

CREATE INDEX lyrics_lookup_wake_idx
    ON lyrics_lookup_state (updated_at, track_id)
    WHERE wake_message_id IS NOT NULL AND wake_durable_at IS NULL;

CREATE TABLE lyrics_lookup_backfill_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    cursor_id uuid,
    rows_seen bigint NOT NULL DEFAULT 0,
    rows_created bigint NOT NULL DEFAULT 0,
    completed boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO lyrics_lookup_backfill_state (singleton)
VALUES (true)
ON CONFLICT (singleton) DO NOTHING;

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
        updated_at = now();

    RETURN NEW;
END
$$;

CREATE TRIGGER tracks_lyrics_lookup_state_refresh
AFTER INSERT OR UPDATE OF
    sc_track_id,
    title,
    metadata_artist,
    uploader_username,
    duration_ms,
    genius_song_id,
    genius_url,
    release_date,
    sc_created_at,
    index_priority
ON tracks
FOR EACH ROW
EXECUTE FUNCTION lyrics_lookup_track_state_refresh();
