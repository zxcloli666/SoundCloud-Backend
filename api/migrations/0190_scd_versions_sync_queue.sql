SET LOCAL lock_timeout = '5s';

ALTER TABLE public.sync_queue
    ADD COLUMN IF NOT EXISTS cancelled boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS exec_generation bigint,
    ADD COLUMN IF NOT EXISTS intent_v bigint NOT NULL DEFAULT 0;
