SELECT sc_track_id,
       urn,
       title,
       artwork_url,
       permalink_url,
       uploader_username,
       uploader_avatar_url
FROM tracks
WHERE sc_track_id = ANY ($1)
