CREATE TABLE IF NOT EXISTS user_like_ranks (
    user_id text NOT NULL,
    sc_track_id text NOT NULL,
    like_created_at timestamptz NOT NULL,
    rank bigint NOT NULL,
    PRIMARY KEY (user_id, sc_track_id)
);

CREATE INDEX IF NOT EXISTS user_like_ranks_user_rank_idx ON user_like_ranks (user_id, rank);
