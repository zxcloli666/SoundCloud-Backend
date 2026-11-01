WITH locked AS MATERIALIZED (SELECT credit.track_id, credit.artist_id, credit.role
                             FROM track_artists AS credit
                             WHERE credit.track_id = $1
                             ORDER BY credit.artist_id, credit.role
                             FOR UPDATE OF credit)
DELETE
FROM track_artists AS credit
    USING locked
WHERE credit.track_id = locked.track_id
  AND credit.artist_id = locked.artist_id
  AND credit.role = locked.role
