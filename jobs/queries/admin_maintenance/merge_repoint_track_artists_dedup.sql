DELETE
FROM track_artists ta USING track_artists h
WHERE ta.artist_id = $1
  AND h.artist_id = $2
  AND h.track_id = ta.track_id
  AND h.role = ta.role
