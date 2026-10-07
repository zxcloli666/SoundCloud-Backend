CREATE TABLE IF NOT EXISTS user_blocked_artists (
    sc_user_id  text        NOT NULL,
    kind        text        NOT NULL CHECK (kind IN ('user', 'artist')),
    target_id   text        NOT NULL,
    name        text        NOT NULL,
    avatar_url  text,
    sc_user_ids text[]      NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (sc_user_id, kind, target_id)
);
