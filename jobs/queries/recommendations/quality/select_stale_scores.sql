SELECT sc_track_id
FROM tracks
WHERE quality_score IS NOT NULL
  AND indexed_at IS NOT NULL
  AND index_state = 'indexed'
  AND storage_state <> 'too_long'
  AND needs_duration_resolve = false
  AND COALESCE(quality_model_version, 0) < $1
ORDER BY COALESCE(quality_model_version, 0), indexed_at DESC
LIMIT $2
