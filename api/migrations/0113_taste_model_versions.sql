CREATE TABLE IF NOT EXISTS taste_model_versions (
    version text PRIMARY KEY,
    input_object text NOT NULL UNIQUE,
    collection text NOT NULL UNIQUE,
    trained_at timestamptz NOT NULL,
    dim smallint NOT NULL CHECK (dim = 128),
    pooling jsonb NOT NULL,
    metrics jsonb NOT NULL,
    items_count integer NOT NULL CHECK (items_count >= 0),
    users_count integer NOT NULL CHECK (users_count >= 0),
    active boolean NOT NULL DEFAULT false,
    applied_at timestamptz,
    refreshed_through timestamp without time zone,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS taste_model_versions_single_active_idx
    ON taste_model_versions ((true))
    WHERE active;

CREATE TABLE IF NOT EXISTS user_taste_vectors (
    sc_user_id text NOT NULL,
    version text NOT NULL REFERENCES taste_model_versions (version) ON DELETE CASCADE,
    vec real[] NOT NULL CHECK (array_length(vec, 1) = 128),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (sc_user_id, version)
);

CREATE INDEX IF NOT EXISTS user_taste_vectors_version_idx
    ON user_taste_vectors (version);

CREATE TABLE IF NOT EXISTS taste_schedule (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    next_export_at timestamptz NOT NULL DEFAULT now(),
    next_refresh_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO taste_schedule (singleton)
VALUES (true)
ON CONFLICT (singleton) DO NOTHING;
