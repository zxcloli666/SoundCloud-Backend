SELECT EXISTS(
    SELECT 1 FROM playlists WHERE urn = $1 AND sharing = 'public' AND deleted_at IS NULL
) AS "public!"
