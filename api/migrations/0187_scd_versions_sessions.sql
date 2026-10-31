SET LOCAL lock_timeout = '5s';

ALTER TABLE public.sessions ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
