ALTER TABLE playlist_legacy_membership_intents
    ADD COLUMN IF NOT EXISTS prior_classification text;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'playlist_legacy_membership_intents'::regclass
          AND conname = 'playlist_legacy_membership_intents_prior_classification_valid'
    ) THEN
        ALTER TABLE playlist_legacy_membership_intents
            ADD CONSTRAINT playlist_legacy_membership_intents_prior_classification_valid
            CHECK (
                prior_classification IS NULL
                OR prior_classification IN (
                    'unclassified',
                    'equal',
                    'remote_superset',
                    'local_superset',
                    'order_only',
                    'membership_diverged'
                )
            ) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'playlist_legacy_membership_intents'::regclass
          AND conname = 'playlist_legacy_membership_intents_resolution_provenance'
    ) THEN
        ALTER TABLE playlist_legacy_membership_intents
            ADD CONSTRAINT playlist_legacy_membership_intents_resolution_provenance
            CHECK (
                classification NOT IN ('resolved', 'abandoned')
                OR prior_classification IS NOT NULL
            ) NOT VALID;
    END IF;
END $$;

ALTER TABLE playlist_legacy_membership_intents
    VALIDATE CONSTRAINT playlist_legacy_membership_intents_prior_classification_valid;

ALTER TABLE playlist_legacy_membership_intents
    VALIDATE CONSTRAINT playlist_legacy_membership_intents_resolution_provenance;

CREATE INDEX IF NOT EXISTS playlist_legacy_membership_intents_drain_idx
    ON playlist_legacy_membership_intents (classification, archived_at, archive_id)
    WHERE resolved_at IS NULL;

DROP INDEX IF EXISTS playlist_legacy_membership_intents_review_idx;
