SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.job_queue_waits (
    kind text NOT NULL,
    lane text NOT NULL,
    due bigint NOT NULL,
    oldest_due_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, lane),
    CONSTRAINT job_queue_waits_lane_ck CHECK (lane IN ('read', 'queue')),
    CONSTRAINT job_queue_waits_due_ck CHECK (due >= 0)
);

CREATE TABLE IF NOT EXISTS public.library_sync_requests (
    sc_user_id text PRIMARY KEY,
    requested_at timestamptz NOT NULL,
    row_v bigint NOT NULL DEFAULT 0
);
