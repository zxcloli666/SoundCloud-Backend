WITH served AS MATERIALIZED (
    SELECT mirror.user_id, mirror.playlist_urn, mirror.wanted_v
    FROM user_likes_playlists AS mirror
    WHERE NOT mirror.wanted_state
      AND mirror.synced_for_v >= mirror.wanted_v
      AND NOT (mirror.wanted_v = 0 AND mirror.progress)
      AND mirror.wanted_v < ((extract(epoch FROM now() - interval '7 days') * 1000000)::bigint << 4)
    LIMIT $1
)
SELECT count(*) AS "purged!"
FROM served
WHERE public.scd_purge_intent(
    'user_likes_playlists',
    jsonb_build_object('user_id', served.user_id, 'playlist_urn', served.playlist_urn),
    served.wanted_v
)
