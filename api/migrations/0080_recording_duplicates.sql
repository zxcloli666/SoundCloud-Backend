ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS superseded_by uuid REFERENCES tracks (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS tracks_superseded_by_idx
    ON tracks (superseded_by)
    WHERE superseded_by IS NOT NULL;

CREATE INDEX IF NOT EXISTS tracks_duplicate_scan_idx
    ON tracks (primary_artist_id, recording_key, id)
    WHERE recording_key IS NOT NULL
        AND primary_artist_id IS NOT NULL
        AND superseded_by IS NULL;

CREATE TABLE IF NOT EXISTS catalog_merge_state
(
    singleton            boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    cursor_artist        uuid,
    cursor_recording_key text,
    passes_completed     bigint NOT NULL DEFAULT 0,
    groups_merged        bigint NOT NULL DEFAULT 0,
    tracks_superseded    bigint NOT NULL DEFAULT 0,
    updated_at           timestamptz,
    CONSTRAINT catalog_merge_state_cursor_pairing
        CHECK ((cursor_artist IS NULL) = (cursor_recording_key IS NULL))
);

INSERT INTO catalog_merge_state (singleton)
VALUES (true)
ON CONFLICT (singleton) DO NOTHING;
