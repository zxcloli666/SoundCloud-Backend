DELETE FROM playlist_track_projection
WHERE playlist_urn = $1
  AND sc_track_id = ANY ($2::text[])
