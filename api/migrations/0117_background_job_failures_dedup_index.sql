CREATE INDEX IF NOT EXISTS background_job_failures_dedup_failed_idx
    ON background_job_failures (kind, dedup_key, failed_at DESC)
    WHERE dedup_key IS NOT NULL;
