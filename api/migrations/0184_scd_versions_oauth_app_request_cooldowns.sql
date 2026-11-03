SET LOCAL lock_timeout = '5s';

ALTER TABLE public.oauth_app_request_cooldowns ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
