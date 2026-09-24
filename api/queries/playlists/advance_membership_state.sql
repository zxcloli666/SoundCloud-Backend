UPDATE playlist_membership_state
SET last_operation_sequence = $2,
    projection_revision = $3,
    projection_track_count = $4,
    sync_status = 'pending',
    conflict_code = NULL,
    candidate_fingerprint = NULL,
    next_reconcile_at = clock_timestamp() + make_interval(secs => $5::double precision),
    updated_at = clock_timestamp()
WHERE playlist_urn = $1
