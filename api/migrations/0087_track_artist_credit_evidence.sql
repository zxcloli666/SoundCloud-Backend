ALTER TABLE track_artists
    ADD COLUMN IF NOT EXISTS evidence varchar(24) NOT NULL DEFAULT 'unattributed';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'track_artists'::regclass
          AND conname = 'track_artists_evidence_valid'
    ) THEN
        ALTER TABLE track_artists
            ADD CONSTRAINT track_artists_evidence_valid
            CHECK (evidence IN (
                'unattributed',
                'external_id',
                'verified_account',
                'external_credit',
                'metadata_field',
                'title_heuristic',
                'ai_inference',
                'uploader_name',
                'manual'
            )) NOT VALID;
    END IF;
END $$;

ALTER TABLE track_artists
    VALIDATE CONSTRAINT track_artists_evidence_valid;

CREATE INDEX IF NOT EXISTS track_artists_weak_evidence_idx
    ON track_artists (evidence, track_id)
    WHERE evidence IN ('uploader_name', 'title_heuristic', 'ai_inference', 'unattributed');
