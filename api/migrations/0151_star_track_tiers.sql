CREATE TABLE IF NOT EXISTS star_track_tiers (
    sc_track_id text PRIMARY KEY,
    tier smallint NOT NULL,
    tiered_at timestamptz NOT NULL DEFAULT now(),
    audio_due boolean NOT NULL DEFAULT true,
    lookup_due boolean NOT NULL DEFAULT true,
    embed_due boolean NOT NULL DEFAULT true,
    align_due boolean NOT NULL DEFAULT true,
    enrich_due boolean NOT NULL DEFAULT true,
    upgrade_from text,
    upgrade_tried_at timestamptz,
    CONSTRAINT star_track_tiers_tier_valid CHECK (tier IN (1, 3))
);

CREATE INDEX IF NOT EXISTS star_track_tiers_audio_due_idx
    ON star_track_tiers (tier, tiered_at, sc_track_id)
    WHERE audio_due;

CREATE INDEX IF NOT EXISTS star_track_tiers_lookup_due_idx
    ON star_track_tiers (tier, tiered_at, sc_track_id)
    WHERE lookup_due;

CREATE INDEX IF NOT EXISTS star_track_tiers_embed_due_idx
    ON star_track_tiers (tier, tiered_at, sc_track_id)
    WHERE embed_due;

CREATE INDEX IF NOT EXISTS star_track_tiers_align_due_idx
    ON star_track_tiers (tier, tiered_at, sc_track_id)
    WHERE align_due;

CREATE INDEX IF NOT EXISTS star_track_tiers_enrich_due_idx
    ON star_track_tiers (tier, tiered_at, sc_track_id)
    WHERE enrich_due;

CREATE INDEX IF NOT EXISTS star_track_tiers_upgrade_idx
    ON star_track_tiers (tier, upgrade_from, upgrade_tried_at NULLS FIRST, tiered_at, sc_track_id)
    WHERE upgrade_from IS NOT NULL;
