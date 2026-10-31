UPDATE artists
SET genius_artist_id   = $2,
    genius_next_run_at = now(),
    genius_locked_at   = NULL,
    updated_at         = now()
WHERE id = $1
  AND genius_artist_id IS NULL
  AND NOT EXISTS (
      SELECT 1 FROM artists other
      WHERE other.genius_artist_id = $2 AND other.id <> $1
  )
