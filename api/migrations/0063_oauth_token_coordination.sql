CREATE TABLE oauth_app_token_refresh_state (
    oauth_app_id uuid PRIMARY KEY REFERENCES oauth_apps(id) ON DELETE CASCADE,
    retry_at timestamptz NOT NULL DEFAULT now(),
    lease_id uuid,
    lease_expires_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT oauth_app_token_refresh_state_lease_valid CHECK (
        (lease_id IS NULL AND lease_expires_at IS NULL)
        OR (lease_id IS NOT NULL AND lease_expires_at IS NOT NULL)
    )
);

CREATE INDEX oauth_app_token_refresh_state_due_idx
    ON oauth_app_token_refresh_state (retry_at, lease_expires_at, oauth_app_id);

CREATE TABLE oauth_app_token_issuance_reservations (
    id uuid PRIMARY KEY,
    oauth_app_id uuid REFERENCES oauth_apps(id) ON DELETE SET NULL,
    client_id text NOT NULL,
    reserved_at timestamptz NOT NULL DEFAULT now(),
    released_at timestamptz
);

CREATE INDEX oauth_app_token_issuance_client_window_idx
    ON oauth_app_token_issuance_reservations (client_id, reserved_at)
    WHERE released_at IS NULL;

CREATE INDEX oauth_app_token_issuance_egress_window_idx
    ON oauth_app_token_issuance_reservations (reserved_at)
    WHERE released_at IS NULL;
