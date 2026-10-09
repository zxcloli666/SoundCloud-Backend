WITH due AS (
    SELECT playlist_urn
    FROM playlist_membership_state
    WHERE last_operation_sequence > committed_operation_sequence
      AND next_reconcile_at <= now()
    ORDER BY next_reconcile_at
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp() + make_interval(secs => $2::double precision),
    updated_at = clock_timestamp()
FROM due
WHERE state.playlist_urn = due.playlist_urn
RETURNING state.playlist_urn AS "playlist_urn!"
