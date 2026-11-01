SET LOCAL lock_timeout = '5s';

ALTER TABLE public.soundcloud_connections ADD COLUMN IF NOT EXISTS token_v bigint NOT NULL DEFAULT 0;
