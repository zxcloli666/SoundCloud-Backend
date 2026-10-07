SELECT urn AS "urn!" FROM tracks WHERE urn = $1 OR lower(permalink_url) = ANY($2::text[])
UNION ALL
SELECT urn FROM playlists WHERE urn = $1 OR lower(permalink_url) = ANY($2::text[])
UNION ALL
SELECT urn FROM users WHERE urn = $1 OR lower(permalink_url) = ANY($2::text[])
LIMIT 2
