SELECT sc_track_id
FROM tracks
WHERE indexed_at IS NOT NULL
  AND index_state = 'indexed'
  AND storage_state <> 'too_long'
  AND needs_duration_resolve = false
  AND quality_score IS NULL
ORDER BY indexed_at DESC
LIMIT $1
