SET LOCAL lock_timeout = '5s';

ALTER TABLE public.discover_promoted ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
