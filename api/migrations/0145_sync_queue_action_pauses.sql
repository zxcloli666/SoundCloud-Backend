CREATE TABLE IF NOT EXISTS sync_queue_action_pauses (
    oauth_app_id uuid NOT NULL REFERENCES oauth_apps (id) ON DELETE CASCADE,
    action_type text NOT NULL,
    paused_until timestamptz NOT NULL,
    strikes integer NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (oauth_app_id, action_type),
    CONSTRAINT sync_queue_action_pauses_strikes_valid CHECK (strikes > 0)
);
