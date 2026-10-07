SELECT COUNT(*)                                                 AS "plays!",
       COALESCE(SUM(duration::bigint), 0)::bigint               AS "listened_ms!",
       COUNT(DISTINCT CASE
                          WHEN sc_track_id ~ '^[0-9]+$' THEN 'soundcloud:tracks:' || sc_track_id
                          ELSE sc_track_id
           END)                                                 AS "tracks!",
       COUNT(DISTINCT COALESCE(artist_urn, lower(artist_name))) AS "artists!"
FROM listening_history
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
