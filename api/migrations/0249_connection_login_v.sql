SET LOCAL lock_timeout = '5s';

ALTER TABLE public.soundcloud_connections ADD COLUMN IF NOT EXISTS login_v bigint NOT NULL DEFAULT 0;
