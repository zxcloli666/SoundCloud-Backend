SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_video_pool (
    sc_track_id text PRIMARY KEY,
    score bigint NOT NULL,
    scored_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS track_video_pool_rank_idx
    ON public.track_video_pool (score DESC, sc_track_id);

CREATE INDEX IF NOT EXISTS track_video_pool_scored_idx
    ON public.track_video_pool (scored_at, sc_track_id);

CREATE INDEX IF NOT EXISTS track_video_dispatch_due_open_idx
    ON public.track_video_dispatch (next_at)
    WHERE stage NOT IN ('dispatched', 'pending_vlm', 'vlm_dispatched', 'found');
