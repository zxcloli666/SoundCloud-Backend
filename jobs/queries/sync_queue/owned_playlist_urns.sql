SELECT playlist_urn
FROM user_owned_playlists
WHERE user_id = ANY ($1)
  AND created_at < $2
