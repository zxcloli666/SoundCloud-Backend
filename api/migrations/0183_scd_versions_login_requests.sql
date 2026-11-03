SET LOCAL lock_timeout = '5s';

ALTER TABLE public.login_requests ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
