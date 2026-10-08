CREATE TABLE IF NOT EXISTS rec_hard_negative_sweep (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    scanned_through timestamp NOT NULL,
    scanned_event_id uuid NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
