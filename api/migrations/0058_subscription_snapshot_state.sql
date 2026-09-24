CREATE TABLE subscription_snapshot_state (
    singleton boolean PRIMARY KEY DEFAULT true,
    initialized_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT subscription_snapshot_state_singleton CHECK (singleton)
);
