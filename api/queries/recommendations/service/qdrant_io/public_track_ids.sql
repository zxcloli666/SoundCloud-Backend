SELECT sc_track_id
FROM tracks
WHERE sc_track_id = ANY ($1)
  AND sharing = 'public'
  AND superseded_by IS NULL
  AND index_state = 'indexed'
  AND storage_state <> 'too_long'
  AND needs_duration_resolve = false
