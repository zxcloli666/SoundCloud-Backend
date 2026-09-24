SELECT sharing = 'public' AND deleted_at IS NULL AS "public!"
FROM playlists
WHERE urn = $1
