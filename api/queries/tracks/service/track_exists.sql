SELECT EXISTS (SELECT 1 FROM tracks WHERE sc_track_id = $1) AS "exists!"
