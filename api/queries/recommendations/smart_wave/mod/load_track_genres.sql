SELECT sc_track_id,
       genre
FROM tracks
WHERE sc_track_id = ANY ($1)
