CREATE TABLE background_jobs (
    id uuid PRIMARY KEY,
    kind varchar(96) NOT NULL,
    lane varchar(16) NOT NULL,
    dedup_key text,
    payload jsonb NOT NULL,
    priority smallint NOT NULL DEFAULT 0,
    generation bigint NOT NULL DEFAULT 1,
    attempts integer NOT NULL DEFAULT 0,
    max_attempts smallint NOT NULL DEFAULT 8,
    available_at timestamptz NOT NULL DEFAULT now(),
    lease_id uuid,
    lease_generation bigint,
    leased_by text,
    lease_expires_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT background_jobs_generation_positive CHECK (generation > 0),
    CONSTRAINT background_jobs_lane_valid CHECK (lane IN ('core_fast', 'core_bulk', 'ops')),
    CONSTRAINT background_jobs_attempts_valid CHECK (
        max_attempts > 0 AND attempts BETWEEN 0 AND max_attempts
    ),
    CONSTRAINT background_jobs_dedup_key_valid CHECK (
        dedup_key IS NULL OR octet_length(dedup_key) BETWEEN 1 AND 1024
    ),
    CONSTRAINT background_jobs_payload_size_valid CHECK (
        pg_column_size(payload) <= 4 * 1024 * 1024
    ),
    CONSTRAINT background_jobs_lease_complete CHECK (
        (
            lease_id IS NULL
            AND lease_generation IS NULL
            AND leased_by IS NULL
            AND lease_expires_at IS NULL
        )
        OR
        (
            lease_id IS NOT NULL
            AND lease_generation IS NOT NULL
            AND leased_by IS NOT NULL
            AND lease_expires_at IS NOT NULL
        )
    ),
    CONSTRAINT background_jobs_lease_generation_valid CHECK (
        lease_generation IS NULL OR lease_generation BETWEEN 1 AND generation
    )
);

CREATE TABLE background_job_enqueues (
    id uuid PRIMARY KEY,
    accepted_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX background_job_enqueues_accepted_idx
    ON background_job_enqueues (accepted_at);

CREATE UNIQUE INDEX background_jobs_dedup_idx
    ON background_jobs (kind, dedup_key)
    WHERE dedup_key IS NOT NULL;

CREATE UNIQUE INDEX background_jobs_lease_idx
    ON background_jobs (lease_id)
    WHERE lease_id IS NOT NULL;

CREATE INDEX background_jobs_claim_idx
    ON background_jobs (lane, priority DESC, available_at, created_at, id)
    WHERE attempts < max_attempts AND lease_id IS NULL;

CREATE INDEX background_jobs_oldest_claim_idx
    ON background_jobs (lane, available_at, created_at, priority DESC, id)
    WHERE attempts < max_attempts AND lease_id IS NULL;

CREATE INDEX background_jobs_expired_lease_idx
    ON background_jobs (lane, lease_expires_at, created_at, id)
    WHERE lease_id IS NOT NULL AND attempts < max_attempts;

CREATE INDEX background_jobs_exhausted_idx
    ON background_jobs (lease_expires_at NULLS FIRST, created_at)
    WHERE attempts >= max_attempts;

CREATE TABLE background_job_failures (
    id uuid NOT NULL,
    kind varchar(96) NOT NULL,
    lane varchar(16) NOT NULL,
    dedup_key text,
    payload jsonb NOT NULL,
    priority smallint NOT NULL,
    generation bigint NOT NULL,
    attempts integer NOT NULL,
    max_attempts smallint NOT NULL,
    last_error text NOT NULL,
    created_at timestamptz NOT NULL,
    failed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (id, generation),
    CONSTRAINT background_job_failures_generation_positive CHECK (generation > 0),
    CONSTRAINT background_job_failures_lane_valid CHECK (lane IN ('core_fast', 'core_bulk', 'ops')),
    CONSTRAINT background_job_failures_attempts_valid CHECK (
        max_attempts > 0 AND attempts BETWEEN 1 AND max_attempts
    ),
    CONSTRAINT background_job_failures_dedup_key_valid CHECK (
        dedup_key IS NULL OR octet_length(dedup_key) BETWEEN 1 AND 1024
    ),
    CONSTRAINT background_job_failures_payload_size_valid CHECK (
        pg_column_size(payload) <= 4 * 1024 * 1024
    )
);

CREATE INDEX background_job_failures_kind_failed_idx
    ON background_job_failures (kind, failed_at DESC);

CREATE INDEX background_job_failures_failed_idx
    ON background_job_failures (failed_at, id, generation);

CREATE TABLE background_schedules (
    kind varchar(96) PRIMARY KEY,
    lane varchar(16) NOT NULL,
    interval_seconds integer NOT NULL,
    next_run_at timestamptz NOT NULL,
    priority smallint NOT NULL DEFAULT 0,
    max_attempts smallint NOT NULL DEFAULT 8,
    enabled boolean NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT background_schedules_lane_valid CHECK (lane IN ('core_fast', 'core_bulk', 'ops')),
    CONSTRAINT background_schedules_interval_positive CHECK (interval_seconds > 0),
    CONSTRAINT background_schedules_attempts_positive CHECK (max_attempts > 0)
);

CREATE INDEX background_schedules_due_idx
    ON background_schedules (next_run_at)
    WHERE enabled;
