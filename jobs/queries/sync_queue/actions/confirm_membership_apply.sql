UPDATE playlist_membership_state
SET remote_apply_fingerprint = CASE
        WHEN reconcile_generation = $3 THEN decode($2, 'hex')
        ELSE remote_apply_fingerprint
    END,
    remote_apply_generation = CASE
        WHEN reconcile_generation = $3 THEN $3
        ELSE remote_apply_generation
    END,
    remote_applied_at = CASE
        WHEN reconcile_generation = $3 THEN clock_timestamp()
        ELSE remote_applied_at
    END,
    next_reconcile_at = clock_timestamp(),
    updated_at = clock_timestamp()
WHERE playlist_urn = $1
