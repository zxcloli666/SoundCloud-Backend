SET LOCAL lock_timeout = '5s';

ALTER TABLE public.user_like_ranks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
