SELECT id,
       title,
       description,
       duration_ms,
       isrc,
       metadata_artist,
       uploader_sc_user_id,
       uploader_username,
       enrich_source
FROM tracks
WHERE sc_track_id = $1
