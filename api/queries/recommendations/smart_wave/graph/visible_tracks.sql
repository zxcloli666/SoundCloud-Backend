SELECT sc_track_id AS "sc_track_id!"
FROM tracks
WHERE sc_track_id = ANY ($1)
  AND sharing = 'public'
  AND superseded_by IS NULL
  AND storage_state = 'ok'
  AND index_state = 'indexed'
