SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS admin_catalog_requests (
    request_id uuid PRIMARY KEY,
    command jsonb NOT NULL,
    reply jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS admin_catalog_requests_created_idx ON admin_catalog_requests (created_at);
