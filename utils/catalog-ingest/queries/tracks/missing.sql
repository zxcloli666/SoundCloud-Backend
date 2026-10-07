SELECT wanted AS "sc_track_id!"
FROM unnest($1::text[]) AS wanted
WHERE NOT EXISTS (SELECT 1 FROM tracks WHERE tracks.sc_track_id = wanted)
