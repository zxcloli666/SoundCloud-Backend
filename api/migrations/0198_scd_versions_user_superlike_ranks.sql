SET LOCAL lock_timeout = '5s';

ALTER TABLE public.user_superlike_ranks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
