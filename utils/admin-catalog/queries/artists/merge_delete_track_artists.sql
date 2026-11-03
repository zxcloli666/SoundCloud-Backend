WITH locked AS MATERIALIZED (SELECT credit.track_id, credit.artist_id, credit.role
                             FROM track_artists credit
                             WHERE credit.artist_id = $1
                             ORDER BY credit.track_id, credit.artist_id, credit.role
                                 FOR UPDATE OF credit)
DELETE
FROM track_artists ta
    USING locked
WHERE ta.track_id = locked.track_id
  AND ta.artist_id = locked.artist_id
  AND ta.role = locked.role
