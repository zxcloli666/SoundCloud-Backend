CREATE TABLE oauth_app_request_cooldowns (
    oauth_app_id uuid PRIMARY KEY REFERENCES oauth_apps(id) ON DELETE CASCADE,
    retry_at timestamptz NOT NULL,
    failure_count integer NOT NULL DEFAULT 1,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT oauth_app_request_cooldowns_failure_count_valid CHECK (failure_count > 0)
);

CREATE INDEX oauth_app_request_cooldowns_retry_idx
    ON oauth_app_request_cooldowns (retry_at);
