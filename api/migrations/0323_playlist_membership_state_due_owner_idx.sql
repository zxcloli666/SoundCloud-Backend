-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS playlist_membership_state_due_owner_idx ON playlist_membership_state (next_reconcile_at) INCLUDE (owner_sc_user_id) WHERE sync_status <> 'clean' AND next_reconcile_at IS NOT NULL;
