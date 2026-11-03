SET LOCAL lock_timeout = '5s';

ALTER TABLE public.subscriptions
    ADD COLUMN IF NOT EXISTS star_since bigint;
