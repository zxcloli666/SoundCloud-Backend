SELECT credit.artist_id
FROM track_artists AS credit
WHERE credit.track_id = $1
ORDER BY credit.artist_id, credit.role
    FOR UPDATE OF credit
