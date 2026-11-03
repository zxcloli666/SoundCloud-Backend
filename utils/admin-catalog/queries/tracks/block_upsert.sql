INSERT INTO track_artist_blocks (track_id, artist_id, note)
VALUES ($1, $2, $3)
ON CONFLICT (track_id, artist_id) DO UPDATE SET note = EXCLUDED.note
