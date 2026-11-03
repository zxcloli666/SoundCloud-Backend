SET LOCAL lock_timeout = '5s';

ALTER TABLE public.track_artist_blocks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
