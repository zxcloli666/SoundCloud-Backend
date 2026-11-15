SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_highlights (
    track_id uuid PRIMARY KEY,
    start_ms integer NOT NULL,
    length_ms integer NOT NULL,
    score real NOT NULL,
    version smallint NOT NULL,
    computed_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT track_highlights_window_valid CHECK (start_ms >= 0 AND length_ms > 0),
    CONSTRAINT track_highlights_score_valid CHECK (score >= 0 AND score <= 1),
    CONSTRAINT track_highlights_version_valid CHECK (version > 0)
);

ALTER TABLE public.track_highlights
    ADD CONSTRAINT track_highlights_track_fk
    FOREIGN KEY (track_id) REFERENCES public.tracks(id) ON DELETE CASCADE NOT VALID;

CREATE TABLE IF NOT EXISTS public.track_highlight_dispatch (
    track_id uuid PRIMARY KEY,
    version smallint NOT NULL,
    state text NOT NULL DEFAULT 'queued',
    popularity bigint NOT NULL DEFAULT 0,
    attempt integer NOT NULL DEFAULT 0,
    failures smallint NOT NULL DEFAULT 0,
    next_run_at timestamptz NOT NULL DEFAULT now(),
    dispatched_at timestamptz,
    reason text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT track_highlight_dispatch_state_valid
        CHECK (state IN ('queued', 'dispatched', 'done', 'failed')),
    CONSTRAINT track_highlight_dispatch_counts_valid
        CHECK (version > 0 AND attempt >= 0 AND failures >= 0),
    CONSTRAINT track_highlight_dispatch_flight_valid
        CHECK (state <> 'dispatched' OR dispatched_at IS NOT NULL)
);

ALTER TABLE public.track_highlight_dispatch
    ADD CONSTRAINT track_highlight_dispatch_track_fk
    FOREIGN KEY (track_id) REFERENCES public.tracks(id) ON DELETE CASCADE NOT VALID;

CREATE INDEX IF NOT EXISTS track_highlight_dispatch_queued_idx
    ON public.track_highlight_dispatch (popularity DESC, track_id)
    WHERE state = 'queued';

CREATE INDEX IF NOT EXISTS track_highlight_dispatch_flight_idx
    ON public.track_highlight_dispatch (dispatched_at)
    WHERE state = 'dispatched';
