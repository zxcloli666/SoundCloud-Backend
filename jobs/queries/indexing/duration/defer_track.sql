UPDATE tracks
SET duration_resolve_attempts = LEAST(duration_resolve_attempts + 1, 15),
    duration_resolve_retry_at = now() + LEAST(
        $2::bigint,
        $1::bigint * (1::bigint << LEAST(duration_resolve_attempts::integer, 10))
    ) * interval '1 millisecond',
    updated_at = now()
WHERE sc_track_id = $3
  AND needs_duration_resolve = true
