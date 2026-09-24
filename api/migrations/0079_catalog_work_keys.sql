ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS work_key text,
    ADD COLUMN IF NOT EXISTS recording_key text,
    ADD COLUMN IF NOT EXISTS work_normalizer_version smallint;

CREATE INDEX IF NOT EXISTS tracks_work_key_idx
    ON tracks (work_key, primary_artist_id)
    WHERE work_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS tracks_recording_key_idx
    ON tracks (recording_key, primary_artist_id)
    WHERE recording_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS tracks_work_backfill_idx
    ON tracks (id)
    WHERE work_normalizer_version IS NULL;

CREATE TABLE IF NOT EXISTS track_work_aliases
(
    track_id  uuid NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    alias_key text NOT NULL,
    PRIMARY KEY (track_id, alias_key)
);

CREATE INDEX IF NOT EXISTS track_work_aliases_key_idx
    ON track_work_aliases (alias_key, track_id);

ALTER TABLE wanted_tracks
    ADD COLUMN IF NOT EXISTS work_key text,
    ADD COLUMN IF NOT EXISTS recording_key text,
    ADD COLUMN IF NOT EXISTS work_normalizer_version smallint,
    ADD COLUMN IF NOT EXISTS work_reconciled_at timestamptz;

CREATE INDEX IF NOT EXISTS wanted_tracks_work_backfill_idx
    ON wanted_tracks (id)
    WHERE work_normalizer_version IS NULL;

CREATE INDEX IF NOT EXISTS wanted_tracks_work_reconcile_idx
    ON wanted_tracks (work_reconciled_at NULLS FIRST, id)
    WHERE status = 'wanted' AND track_id IS NULL AND work_key IS NOT NULL
        AND primary_artist_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS wanted_track_work_aliases
(
    wanted_track_id uuid NOT NULL REFERENCES wanted_tracks (id) ON DELETE CASCADE,
    alias_key       text NOT NULL,
    PRIMARY KEY (wanted_track_id, alias_key)
);

CREATE INDEX IF NOT EXISTS wanted_track_work_aliases_key_idx
    ON wanted_track_work_aliases (alias_key, wanted_track_id);

CREATE TABLE IF NOT EXISTS catalog_work_links
(
    wanted_track_id uuid        NOT NULL REFERENCES wanted_tracks (id) ON DELETE CASCADE,
    track_id        uuid        NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    match_reason    varchar(32) NOT NULL,
    matched_key     text        NOT NULL,
    normalizer_version smallint NOT NULL,
    linked_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (wanted_track_id, track_id)
);

CREATE INDEX IF NOT EXISTS catalog_work_links_track_idx
    ON catalog_work_links (track_id);
