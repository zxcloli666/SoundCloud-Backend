SET LOCAL lock_timeout = '5s';

ALTER TABLE public.track_video_dispatch
    ADD COLUMN IF NOT EXISTS payload jsonb;

ALTER TABLE public.video_accounts
    ADD COLUMN IF NOT EXISTS paused_until timestamptz,
    ADD COLUMN IF NOT EXISTS last_leased_at timestamptz;
