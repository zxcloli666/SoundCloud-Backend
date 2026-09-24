CREATE TABLE IF NOT EXISTS sc_egress_health (
    channel text PRIMARY KEY,
    open_until timestamptz NOT NULL,
    opened_by text NOT NULL,
    opened_at timestamptz NOT NULL DEFAULT now()
);
