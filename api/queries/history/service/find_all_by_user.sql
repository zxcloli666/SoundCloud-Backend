SELECT history.id,
       history.soundcloud_user_id,
       history.sc_track_id,
       history.title,
       history.artist_name,
       history.artist_urn,
       history.artwork_url,
       history.duration,
       history.played_at,
       NULL::text AS track_urn
FROM (SELECT DISTINCT user_id FROM unnest($1::text[]) AS user_id) AS users
CROSS JOIN LATERAL (
    SELECT id, soundcloud_user_id, sc_track_id, title, artist_name, artist_urn, artwork_url, duration, played_at
    FROM listening_history
    WHERE soundcloud_user_id = users.user_id
    ORDER BY played_at DESC, id DESC
    LIMIT $2::bigint + $3::bigint
) AS history
ORDER BY history.played_at DESC, history.id DESC
LIMIT $2 OFFSET $3
