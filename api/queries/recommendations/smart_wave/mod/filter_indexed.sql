SELECT sc_track_id
FROM tracks
WHERE sc_track_id = ANY ($1)
  AND superseded_by IS NULL
  AND index_state = 'indexed'
