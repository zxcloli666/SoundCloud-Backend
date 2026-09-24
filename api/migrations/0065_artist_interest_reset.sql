CREATE INDEX IF NOT EXISTS artists_positive_interest_idx
    ON artists (id)
    WHERE interest_score > 0;
