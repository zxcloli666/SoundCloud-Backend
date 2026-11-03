WITH locked AS MATERIALIZED (SELECT ta.track_id, ta.artist_id, ta.role
                             FROM track_artists ta
                             WHERE ta.role = 'feature'
                             ORDER BY ta.track_id, ta.artist_id, ta.role
                             FOR UPDATE OF ta)
UPDATE track_artists ta
SET role = 'featured'
FROM locked
WHERE ta.track_id = locked.track_id
  AND ta.artist_id = locked.artist_id
  AND ta.role = locked.role
