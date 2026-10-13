CREATE INDEX IF NOT EXISTS background_jobs_kind_claim_idx
    ON background_jobs (kind, priority DESC, available_at, created_at, id)
    WHERE attempts < max_attempts AND lease_id IS NULL;
