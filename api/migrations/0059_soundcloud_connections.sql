CREATE TABLE soundcloud_connections (
    id uuid PRIMARY KEY,
    soundcloud_user_id text NOT NULL,
    username text,
    oauth_app_id uuid REFERENCES oauth_apps(id) ON DELETE RESTRICT,
    uses_environment_oauth_app boolean NOT NULL DEFAULT false,
    access_token text NOT NULL,
    refresh_token text NOT NULL,
    expires_at timestamptz NOT NULL,
    scope text NOT NULL,
    refresh_generation bigint NOT NULL DEFAULT 1,
    refresh_failure_count integer NOT NULL DEFAULT 0,
    refresh_lease_id uuid,
    refresh_lease_expires_at timestamptz,
    last_refresh_attempt_at timestamptz,
    last_refresh_success_at timestamptz,
    last_refresh_error_kind varchar(32),
    last_refresh_error text,
    retry_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT soundcloud_connections_access_token_present CHECK (octet_length(access_token) > 0),
    CONSTRAINT soundcloud_connections_refresh_token_present CHECK (octet_length(refresh_token) > 0),
    CONSTRAINT soundcloud_connections_generation_positive CHECK (refresh_generation > 0),
    CONSTRAINT soundcloud_connections_failure_count_valid CHECK (refresh_failure_count >= 0),
    CONSTRAINT soundcloud_connections_oauth_source_valid CHECK (
        oauth_app_id IS NULL OR uses_environment_oauth_app = false
    ),
    CONSTRAINT soundcloud_connections_refresh_lease_complete CHECK (
        (refresh_lease_id IS NULL AND refresh_lease_expires_at IS NULL)
        OR
        (refresh_lease_id IS NOT NULL AND refresh_lease_expires_at IS NOT NULL)
    ),
    CONSTRAINT soundcloud_connections_refresh_error_valid CHECK (
        last_refresh_error_kind IS NULL
        OR last_refresh_error_kind IN (
            'rate_limited',
            'reauthorization_required',
            'temporarily_unavailable',
            'timed_out'
        )
    )
);

ALTER TABLE sessions
    ADD COLUMN soundcloud_connection_id uuid;

UPDATE sessions
SET soundcloud_connection_id = id
WHERE soundcloud_user_id IS NOT NULL
  AND btrim(soundcloud_user_id) <> ''
  AND access_token <> ''
  AND refresh_token <> '';

INSERT INTO soundcloud_connections (
    id,
    soundcloud_user_id,
    username,
    oauth_app_id,
    uses_environment_oauth_app,
    access_token,
    refresh_token,
    expires_at,
    scope,
    last_refresh_success_at,
    created_at,
    updated_at
)
SELECT session.soundcloud_connection_id,
       CASE
           WHEN session.soundcloud_user_id ~ '^soundcloud:users:[0-9]+$'
               THEN split_part(session.soundcloud_user_id, ':', 3)
           ELSE btrim(session.soundcloud_user_id)
       END,
       session.username,
       oauth_app.id,
       session.oauth_app_id IS NULL OR btrim(session.oauth_app_id) = '',
       session.access_token,
       session.refresh_token,
       session.expires_at AT TIME ZONE 'UTC',
       session.scope,
       session.updated_at AT TIME ZONE 'UTC',
       session.created_at AT TIME ZONE 'UTC',
       session.updated_at AT TIME ZONE 'UTC'
FROM sessions AS session
LEFT JOIN oauth_apps AS oauth_app ON oauth_app.id::text = session.oauth_app_id
WHERE session.soundcloud_connection_id IS NOT NULL;

WITH aliases AS (
    SELECT id,
           first_value(id) OVER (
               PARTITION BY
                   soundcloud_user_id,
                   oauth_app_id,
                   uses_environment_oauth_app,
                   refresh_token
               ORDER BY updated_at DESC, id
           ) AS canonical_id
    FROM soundcloud_connections
)
UPDATE sessions AS session
SET soundcloud_connection_id = aliases.canonical_id
FROM aliases
WHERE session.soundcloud_connection_id = aliases.id;

DELETE FROM soundcloud_connections AS connection
WHERE NOT EXISTS (
    SELECT 1
    FROM sessions AS session
    WHERE session.soundcloud_connection_id = connection.id
);

ALTER TABLE sessions
    ADD CONSTRAINT sessions_soundcloud_connection_fk
        FOREIGN KEY (soundcloud_connection_id)
        REFERENCES soundcloud_connections(id)
        ON DELETE SET NULL;

CREATE INDEX sessions_soundcloud_connection_idx
    ON sessions (soundcloud_connection_id);

CREATE INDEX soundcloud_connections_expiry_idx
    ON soundcloud_connections (expires_at);

CREATE INDEX soundcloud_connections_user_idx
    ON soundcloud_connections (soundcloud_user_id, updated_at DESC);

CREATE INDEX soundcloud_connections_refresh_lease_idx
    ON soundcloud_connections (refresh_lease_expires_at)
    WHERE refresh_lease_id IS NOT NULL;

UPDATE background_schedules
SET enabled = false,
    updated_at = now()
WHERE kind = 'auth.reap_sessions';

DELETE FROM background_jobs
WHERE kind = 'auth.reap_sessions';

DROP INDEX IF EXISTS sessions_reap_idx;

ALTER TABLE sessions
    DROP COLUMN access_token,
    DROP COLUMN refresh_token,
    DROP COLUMN expires_at,
    DROP COLUMN scope,
    DROP COLUMN soundcloud_user_id,
    DROP COLUMN username,
    DROP COLUMN oauth_app_id;
