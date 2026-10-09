SELECT (array_agg(artist_name ORDER BY played_at DESC))[1]                                    AS "artist_name!",
       MAX(artist_urn)                                                                         AS "artist_urn?",
       (array_agg(artwork_url ORDER BY played_at DESC) FILTER (WHERE artwork_url IS NOT NULL))[1] AS "artwork_url?",
       COUNT(*)                                                                                AS "plays!",
       COUNT(DISTINCT CASE
                          WHEN sc_track_id ~ '^[0-9]+$' THEN 'soundcloud:tracks:' || sc_track_id
                          ELSE sc_track_id
                          END)                                                             AS "tracks!",
       COALESCE(SUM(duration::bigint), 0)::bigint                                              AS "listened_ms!"
FROM listening_history
WHERE soundcloud_user_id = ANY ($1)
  AND played_at >= $2
  AND played_at < $3
  AND (artist_urn IS NOT NULL OR artist_name <> '')
GROUP BY COALESCE(artist_urn, lower(artist_name))
ORDER BY COUNT(*) DESC, MAX(played_at) DESC
LIMIT $4
