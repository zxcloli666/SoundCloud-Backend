SELECT candidate.sc_track_id AS "sc_track_id!"
FROM unnest($1::text[]) AS candidate(sc_track_id)
WHERE NOT EXISTS (
    SELECT 1 FROM tracks WHERE tracks.sc_track_id = candidate.sc_track_id
)
ORDER BY candidate.sc_track_id
LIMIT 200
