UPDATE user_likes_tracks AS mirror
SET liked_at = item.liked_at
FROM unnest($2::text[], $3::timestamptz[]) AS item(sc_track_id, liked_at)
WHERE mirror.user_id = ANY ($1::text[])
  AND mirror.sc_track_id = item.sc_track_id
  AND mirror.liked_at IS DISTINCT FROM item.liked_at
