CREATE TABLE IF NOT EXISTS star_track_tiers (
    sc_track_id text PRIMARY KEY,
    tier smallint NOT NULL,
    tiered_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT star_track_tiers_tier_valid CHECK (tier BETWEEN 1 AND 4)
);

CREATE INDEX IF NOT EXISTS star_track_tiers_tier_idx
    ON star_track_tiers (tier, tiered_at, sc_track_id);
