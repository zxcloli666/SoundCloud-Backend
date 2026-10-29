CREATE TABLE diag_bundles (
    code text PRIMARY KEY,
    at timestamptz NOT NULL DEFAULT now(),
    app_version text NOT NULL,
    os text NOT NULL,
    body jsonb NOT NULL
);

CREATE INDEX diag_bundles_at_idx ON diag_bundles (at);

CREATE TABLE diag_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    at timestamptz NOT NULL DEFAULT now(),
    fingerprint text NOT NULL,
    kind text NOT NULL,
    title text NOT NULL,
    app_version text NOT NULL,
    os text NOT NULL,
    install text NOT NULL,
    trace text,
    body jsonb NOT NULL
);

CREATE INDEX diag_events_fingerprint_at_idx ON diag_events (fingerprint, at);
CREATE INDEX diag_events_at_idx ON diag_events (at);
