SET LOCAL lock_timeout = '5s';

ALTER TABLE public.user_likes_tracks
    ADD COLUMN IF NOT EXISTS synced_for_v bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS wanted_v bigint NOT NULL DEFAULT 0;
