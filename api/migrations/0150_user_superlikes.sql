CREATE TABLE IF NOT EXISTS user_superlikes (
    user_id         text        NOT NULL,
    sc_track_id     text        NOT NULL,
    like_created_at timestamptz NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, sc_track_id)
);
CREATE INDEX IF NOT EXISTS user_superlikes_user_created_idx
    ON user_superlikes (user_id, created_at DESC, sc_track_id DESC);
