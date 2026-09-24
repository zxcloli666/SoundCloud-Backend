SELECT it.sc_track_id
FROM tracks it
WHERE it.sc_track_id = ANY ($1)
  AND it.superseded_by IS NULL
  AND it.primary_artist_id = $2
