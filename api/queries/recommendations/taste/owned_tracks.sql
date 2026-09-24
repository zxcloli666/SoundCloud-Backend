SELECT regexp_replace(sc_track_id, '^.*:', '') AS "sc_track_id!"
FROM user_likes_tracks
WHERE user_id = ANY ($1::text[])
  AND wanted_state
  AND regexp_replace(sc_track_id, '^.*:', '') = ANY ($2::text[])
UNION
SELECT regexp_replace(sc_track_id, '^.*:', '')
FROM user_events
WHERE sc_user_id = ANY ($1::text[])
  AND event_type IN ('like', 'playlist_add', 'full_play', 'skip')
  AND regexp_replace(sc_track_id, '^.*:', '') = ANY ($2::text[])
