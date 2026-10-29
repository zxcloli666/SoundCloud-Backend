SELECT COUNT(DISTINCT it.id) ::int8 AS "count!"
FROM tracks it
         JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
WHERE (it.uploader_sc_user_id = $1 OR (it.id = $3 AND it.uploader_sc_user_id IS NULL))
  AND ta.artist_id = $2
