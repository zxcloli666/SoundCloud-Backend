CREATE TABLE IF NOT EXISTS discover_tag_counts
(
    tag          text PRIMARY KEY,
    artist_count bigint      NOT NULL,
    refreshed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS discover_tag_counts_rank_idx
    ON discover_tag_counts (artist_count DESC, tag);
