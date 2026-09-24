CREATE TABLE oauth_apps (
    id uuid PRIMARY KEY,
    active boolean NOT NULL DEFAULT true
);

CREATE TABLE sessions (
    id uuid PRIMARY KEY,
    access_token text NOT NULL,
    refresh_token text NOT NULL,
    expires_at timestamp NOT NULL,
    scope text NOT NULL,
    soundcloud_user_id text,
    username text,
    oauth_app_id text,
    created_at timestamp NOT NULL DEFAULT now(),
    updated_at timestamp NOT NULL DEFAULT now()
);

CREATE INDEX sessions_reap_idx ON sessions (expires_at, updated_at);

CREATE TABLE background_schedules (
    kind text PRIMARY KEY,
    enabled boolean NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE background_jobs (
    id uuid PRIMARY KEY,
    kind text NOT NULL,
    lease_id uuid
);
