SELECT EXISTS(SELECT 1 FROM playlists WHERE urn = $1 AND deleted_at IS NOT NULL) AS "deleted!"
