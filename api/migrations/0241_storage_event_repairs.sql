SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS storage_event_repairs (
    sc_track_id text PRIMARY KEY REFERENCES tracks(sc_track_id) ON DELETE CASCADE,
    uploaded_at timestamp NOT NULL,
    repaired_at timestamptz NOT NULL DEFAULT now()
);
