SELECT COUNT(*) ::int8 AS "count!"
FROM (SELECT it.id
      FROM tracks it
               JOIN track_artists ta ON ta.track_id = it.id AND ta.role = 'primary'
      WHERE it.uploader_sc_user_id = $1
        AND ta.artist_id = $2
      LIMIT 3) AS capped
