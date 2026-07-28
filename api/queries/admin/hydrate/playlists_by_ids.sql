SELECT sc_playlist_id,
       title,
       artwork_url,
       permalink_url
FROM playlists
WHERE sc_playlist_id = ANY ($1)
