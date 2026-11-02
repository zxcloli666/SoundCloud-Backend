SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.lyrics_lookup_queue (
    track_id uuid PRIMARY KEY REFERENCES public.lyrics_lookup_state(track_id) ON DELETE CASCADE,
    priority smallint NOT NULL,
    next_run_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL,
    wake_at timestamptz,
    attempts bigint NOT NULL DEFAULT 0,
    last_attempt_at timestamptz,
    claim_job_id uuid,
    claim_job_generation bigint,
    claim_job_lease_id uuid,
    claim_state_generation bigint,
    claim_expires_at timestamptz,
    CONSTRAINT lyrics_lookup_queue_attempts_valid CHECK (attempts >= 0),
    CONSTRAINT lyrics_lookup_queue_claim_valid CHECK (
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

CREATE INDEX IF NOT EXISTS lyrics_lookup_queue_due_priority_idx
    ON public.lyrics_lookup_queue (priority, next_run_at, created_at, track_id)
    WHERE claim_job_id IS NULL;

CREATE INDEX IF NOT EXISTS lyrics_lookup_queue_due_oldest_idx
    ON public.lyrics_lookup_queue (next_run_at, created_at, track_id)
    WHERE claim_job_id IS NULL;

CREATE INDEX IF NOT EXISTS lyrics_lookup_queue_expired_claim_idx
    ON public.lyrics_lookup_queue (claim_expires_at, track_id)
    WHERE claim_job_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS lyrics_lookup_queue_wake_idx
    ON public.lyrics_lookup_queue (wake_at, track_id)
    WHERE wake_at IS NOT NULL;

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
    INSERT INTO public.lyrics_lookup_queue (
        track_id,
        priority,
        next_run_at,
        created_at,
        wake_at,
        attempts,
        last_attempt_at,
        claim_job_id,
        claim_job_generation,
        claim_job_lease_id,
        claim_state_generation,
        claim_expires_at
    )
    VALUES (
        NEW.track_id,
        NEW.priority,
        NEW.next_run_at,
        NEW.created_at,
        open_wake,
        NEW.attempts,
        NEW.last_attempt_at,
        NEW.claim_job_id,
        NEW.claim_job_generation,
        NEW.claim_job_lease_id,
        NEW.claim_state_generation,
        NEW.claim_expires_at
    )
    ON CONFLICT (track_id) DO NOTHING;
    IF FOUND THEN
        RETURN NULL;
    END IF;

    UPDATE public.lyrics_lookup_queue AS queue
    SET priority = NEW.priority,
        next_run_at = NEW.next_run_at,
        created_at = NEW.created_at,
        wake_at = open_wake,
        attempts = NEW.attempts,
        last_attempt_at = NEW.last_attempt_at,
        claim_job_id = NEW.claim_job_id,
        claim_job_generation = NEW.claim_job_generation,
        claim_job_lease_id = NEW.claim_job_lease_id,
        claim_state_generation = NEW.claim_state_generation,
        claim_expires_at = NEW.claim_expires_at
    WHERE queue.track_id = NEW.track_id
      AND (
          queue.priority,
          queue.next_run_at,
          queue.created_at,
          queue.wake_at,
          queue.attempts,
          queue.last_attempt_at,
          queue.claim_job_id,
          queue.claim_job_generation,
          queue.claim_job_lease_id,
          queue.claim_state_generation,
          queue.claim_expires_at
      ) IS DISTINCT FROM (
          NEW.priority,
          NEW.next_run_at,
          NEW.created_at,
          open_wake,
          NEW.attempts,
          NEW.last_attempt_at,
          NEW.claim_job_id,
          NEW.claim_job_generation,
          NEW.claim_job_lease_id,
          NEW.claim_state_generation,
          NEW.claim_expires_at
      );
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER lyrics_lookup_queue_sync
AFTER INSERT OR UPDATE ON public.lyrics_lookup_state
FOR EACH ROW
WHEN (public.scd_derived())
EXECUTE FUNCTION public.lyrics_lookup_queue_sync();

ALTER TABLE public.lyrics_lookup_state ENABLE ALWAYS TRIGGER lyrics_lookup_queue_sync;

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue')
    );
