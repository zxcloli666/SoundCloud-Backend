UPDATE wanted_tracks
SET resolve_locked_at   = NULL,
    resolve_attempts    = GREATEST(resolve_attempts - 1, 0),
    resolve_next_run_at = now() + $2 * interval '1 second'
WHERE id = ANY ($1)
  AND status = 'wanted'
  AND track_id IS NULL
