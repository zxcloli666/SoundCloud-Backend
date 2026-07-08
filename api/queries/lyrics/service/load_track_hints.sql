SELECT title, duration_ms, metadata_artist, uploader_username, genius_song_id, genius_url
FROM tracks
WHERE sc_track_id = $1
