ALTER TABLE playlist_membership_state
    ADD COLUMN IF NOT EXISTS remote_apply_fingerprint bytea,
    ADD COLUMN IF NOT EXISTS remote_apply_generation bigint,
    ADD COLUMN IF NOT EXISTS remote_applied_at timestamptz;

ALTER TABLE playlist_membership_state
    DROP CONSTRAINT IF EXISTS playlist_membership_state_remote_apply_complete;

ALTER TABLE playlist_membership_state
    ADD CONSTRAINT playlist_membership_state_remote_apply_complete CHECK (
        (remote_apply_fingerprint IS NULL
         AND remote_apply_generation IS NULL
         AND remote_applied_at IS NULL)
        OR
        (remote_apply_fingerprint IS NOT NULL
         AND remote_apply_generation IS NOT NULL
         AND remote_applied_at IS NOT NULL)
    );
