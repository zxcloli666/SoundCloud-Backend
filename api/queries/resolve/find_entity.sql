SELECT found.urn AS "urn!"
FROM (
    SELECT urn FROM tracks WHERE urn = $1
    UNION
    SELECT urn FROM tracks
    WHERE permalink_url IS NOT NULL AND lower(permalink_url) = ANY($2::text[])
    UNION
    SELECT urn FROM playlists WHERE urn = $1
    UNION
    SELECT urn FROM playlists
    WHERE permalink_url IS NOT NULL AND lower(permalink_url) = ANY($2::text[])
    UNION
    SELECT urn FROM users WHERE urn = $1
    UNION
    SELECT urn FROM users
    WHERE permalink_url IS NOT NULL AND lower(permalink_url) = ANY($2::text[])
) AS found
LIMIT 2
