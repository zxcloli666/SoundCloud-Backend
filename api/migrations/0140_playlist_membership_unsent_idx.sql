CREATE INDEX IF NOT EXISTS playlist_membership_state_unsent_idx
    ON playlist_membership_state (next_reconcile_at)
    WHERE last_operation_sequence > committed_operation_sequence;
