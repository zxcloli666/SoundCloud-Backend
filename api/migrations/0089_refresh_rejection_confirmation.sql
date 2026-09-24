ALTER TABLE soundcloud_connections
    ADD COLUMN IF NOT EXISTS refresh_rejection_count integer NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS first_refresh_rejection_at timestamptz;

ALTER TABLE soundcloud_connections
    ADD CONSTRAINT soundcloud_connections_rejection_evidence_valid CHECK (
        (refresh_rejection_count = 0 AND first_refresh_rejection_at IS NULL)
        OR (refresh_rejection_count > 0 AND first_refresh_rejection_at IS NOT NULL)
    ) NOT VALID;

UPDATE soundcloud_connections
SET last_refresh_error_kind = 'temporarily_unavailable',
    last_refresh_error = 'SoundCloud refresh rejection needs confirmation',
    retry_at = now(),
    updated_at = now()
WHERE last_refresh_error_kind = 'reauthorization_required';
