SET LOCAL lock_timeout = '5s';

ALTER TABLE public.featured_items ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
