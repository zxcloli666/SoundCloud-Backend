SET LOCAL lock_timeout = '5s';

ALTER TABLE public.discover_settings ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
