ALTER TABLE playlist_membership_state
    ADD COLUMN IF NOT EXISTS reconcile_failure_streak integer NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'playlist_membership_state'::regclass
          AND conname = 'playlist_membership_state_failure_streak_nonnegative'
    ) THEN
        ALTER TABLE playlist_membership_state
            ADD CONSTRAINT playlist_membership_state_failure_streak_nonnegative
            CHECK (reconcile_failure_streak >= 0) NOT VALID;
    END IF;
END $$;

ALTER TABLE playlist_membership_state
    VALIDATE CONSTRAINT playlist_membership_state_failure_streak_nonnegative;

CREATE INDEX IF NOT EXISTS playlist_membership_operations_conflict_idx
    ON playlist_membership_operations (playlist_urn)
    WHERE outcome = 'conflict';
