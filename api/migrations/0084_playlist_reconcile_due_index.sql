CREATE INDEX IF NOT EXISTS playlist_membership_state_reconcile_due_idx
    ON playlist_membership_state (next_reconcile_at, playlist_urn)
    WHERE sync_status <> 'clean';

DROP INDEX IF EXISTS playlist_membership_state_due_idx;
