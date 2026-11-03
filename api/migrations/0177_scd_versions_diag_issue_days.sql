SET LOCAL lock_timeout = '5s';

ALTER TABLE public.diag_issue_days ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
