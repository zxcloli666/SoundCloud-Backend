CREATE TABLE IF NOT EXISTS user_web_profiles (
    sc_user_id text PRIMARY KEY CHECK (sc_user_id ~ '^[1-9][0-9]*$'),
    profiles jsonb NOT NULL CHECK (
        jsonb_typeof(profiles) = 'array'
        AND jsonb_array_length(profiles) < 200
        AND octet_length(profiles::text) <= 262144
    ),
    sc_observation bigint NOT NULL CHECK (sc_observation > 0),
    synced_at timestamptz NOT NULL DEFAULT now()
);
