SELECT wanted AS "id!"
FROM unnest($1::text[]) wanted
WHERE NOT EXISTS (SELECT 1 FROM tracks WHERE sc_track_id = wanted)
LIMIT 25
