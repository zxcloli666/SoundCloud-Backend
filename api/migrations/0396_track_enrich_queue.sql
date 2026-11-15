SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_enrich_queue (
    track_id uuid PRIMARY KEY,
    enrich_state varchar(16) NOT NULL DEFAULT 'pending',
    index_priority smallint NOT NULL DEFAULT 5,
    next_run_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    attempts smallint NOT NULL DEFAULT 0,
    error text
);

ALTER TABLE public.track_enrich_queue
    ADD CONSTRAINT track_enrich_queue_track_fk
    FOREIGN KEY (track_id) REFERENCES public.tracks(id) ON DELETE CASCADE NOT VALID;

CREATE INDEX IF NOT EXISTS track_enrich_queue_claim_idx
    ON public.track_enrich_queue (index_priority, next_run_at)
    WHERE enrich_state IN ('pending', 'failed');
