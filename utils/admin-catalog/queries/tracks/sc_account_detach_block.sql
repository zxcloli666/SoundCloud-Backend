INSERT INTO track_artist_blocks (track_id, artist_id, note)
SELECT t.id, $1, $2
FROM tracks t
WHERE t.uploader_sc_user_id = $3
  AND (t.primary_artist_id = $1
    OR EXISTS (SELECT 1
               FROM track_artists ta
               WHERE ta.track_id = t.id
                 AND ta.artist_id = $1))
ORDER BY t.id
ON CONFLICT (track_id, artist_id) DO UPDATE SET note = EXCLUDED.note
