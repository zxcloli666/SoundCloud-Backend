UPDATE tracks
SET duration_ms = $2::integer,
    needs_duration_resolve = false,
    duration_resolve_attempts = 0,
    duration_resolve_retry_at = NULL,
    storage_state = CASE
        WHEN $2::integer > $3::integer THEN 'too_long'
        WHEN storage_state = 'too_long' THEN 'pending'
        WHEN storage_state = 'failed' AND duration_ms <> $2::integer THEN 'pending'
        ELSE storage_state
    END,
    index_state = CASE
        WHEN $2::integer > $3::integer THEN 'too_long'
        WHEN index_state = 'too_long' THEN 'pending'
        ELSE index_state
    END,
    indexed_at = CASE
        WHEN $2::integer > $3::integer
             OR index_state <> 'indexed'
             OR storage_state = 'too_long' THEN NULL
        ELSE indexed_at
    END,
    transcribe_state = CASE
        WHEN $2::integer > $3::integer THEN 'disabled'
        WHEN storage_state = 'too_long' THEN NULL
        ELSE transcribe_state
    END,
    hq_upgrade_pending = CASE WHEN $2::integer > $3::integer THEN false ELSE hq_upgrade_pending END,
    storage_attempts = CASE
        WHEN $2::integer <= $3::integer
             AND (storage_state = 'too_long'
                  OR (storage_state = 'failed' AND duration_ms <> $2::integer)) THEN 0
        ELSE storage_attempts
    END,
    updated_at = now()
WHERE sc_track_id = $1
  AND needs_duration_resolve = true
