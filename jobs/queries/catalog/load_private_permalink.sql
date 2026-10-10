SELECT permalink_url
FROM playlists
WHERE urn = $1 AND $2
UNION ALL
SELECT permalink_url
FROM tracks
WHERE urn = $1 AND NOT $2
