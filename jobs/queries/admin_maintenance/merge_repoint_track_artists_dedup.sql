WITH locked AS MATERIALIZED (SELECT ta.track_id, ta.artist_id, ta.role
                             FROM track_artists ta
                             WHERE ta.artist_id = $1
                               AND EXISTS (SELECT 1
                                           FROM track_artists h
                                           WHERE h.artist_id = $2
                                             AND h.track_id = ta.track_id
                                             AND h.role = ta.role)
                             ORDER BY ta.track_id, ta.artist_id, ta.role
                             FOR UPDATE OF ta)
DELETE
FROM track_artists ta USING locked
WHERE ta.track_id = locked.track_id
  AND ta.artist_id = locked.artist_id
  AND ta.role = locked.role
