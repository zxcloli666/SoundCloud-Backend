DELETE
FROM track_artists ta
    USING tracks t
WHERE ta.track_id = t.id
  AND ta.artist_id = $1
  AND t.uploader_sc_user_id = $2
