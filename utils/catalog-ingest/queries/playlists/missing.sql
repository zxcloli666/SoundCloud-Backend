SELECT wanted AS "urn!"
FROM unnest($1::text[]) AS wanted
WHERE NOT EXISTS (SELECT 1 FROM playlists WHERE playlists.urn = wanted)
