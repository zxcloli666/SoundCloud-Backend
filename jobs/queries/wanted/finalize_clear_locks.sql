UPDATE wanted_tracks
SET resolve_locked_at = NULL
WHERE id = ANY ($1)
  AND resolve_locked_at IS NOT NULL
