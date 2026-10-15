CREATE TABLE IF NOT EXISTS user_superlike_ranks (
    user_id              text        NOT NULL,
    sc_track_id          text        NOT NULL,
    superlike_created_at timestamptz NOT NULL,
    rank                 bigint      NOT NULL,
    PRIMARY KEY (user_id, sc_track_id)
);
