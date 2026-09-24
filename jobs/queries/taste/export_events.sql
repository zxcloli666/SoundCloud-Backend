WITH window_events AS (
    SELECT regexp_replace(sc_user_id, '^.*:', '') AS user_id,
           regexp_replace(sc_track_id, '^.*:', '') AS track_id,
           event_type,
           weight,
           created_at
    FROM user_events
    WHERE created_at >= (now() AT TIME ZONE 'UTC') - $1::int * interval '1 day'
      AND event_type IN ('like', 'playlist_add', 'full_play', 'skip')
      AND (event_type <> 'skip' OR weight < 0)
),
withdrawn_likes AS (
    SELECT regexp_replace(user_id, '^.*:', '') AS user_id,
           regexp_replace(sc_track_id, '^.*:', '') AS track_id
    FROM user_likes_tracks
    WHERE NOT wanted_state
),
timed AS (
    SELECT e.user_id,
           e.track_id,
           CASE e.event_type
               WHEN 'like' THEN 0
               WHEN 'playlist_add' THEN 2
               WHEN 'full_play' THEN 3
               ELSE 4
           END AS event_code,
           extract(epoch FROM e.created_at)::bigint AS unix_s,
           e.weight
    FROM window_events e
    WHERE e.event_type <> 'like'
       OR NOT EXISTS (
           SELECT 1
           FROM withdrawn_likes w
           WHERE w.user_id = e.user_id
             AND w.track_id = e.track_id
       )
),
imported AS (
    SELECT regexp_replace(user_id, '^.*:', '') AS user_id,
           regexp_replace(sc_track_id, '^.*:', '') AS track_id,
           CASE
               WHEN liked_at >= now() - $1::int * interval '1 day' AND liked_at <= now()
                   THEN extract(epoch FROM liked_at)::bigint
           END AS liked_unix_s
    FROM user_likes_tracks
    WHERE wanted_state
),
events AS (
    SELECT user_id, track_id, event_code, unix_s, weight
    FROM timed
    UNION ALL
    SELECT i.user_id,
           i.track_id,
           CASE WHEN i.liked_unix_s IS NULL THEN 1 ELSE 0 END,
           i.liked_unix_s,
           1.0::float8
    FROM imported i
    WHERE NOT EXISTS (
        SELECT 1
        FROM timed t
        WHERE t.event_code = 0
          AND t.user_id = i.user_id
          AND t.track_id = i.track_id
    )
    UNION ALL
    SELECT regexp_replace(sc_user_id, '^.*:', ''),
           regexp_replace(sc_track_id, '^.*:', ''),
           5,
           extract(epoch FROM created_at)::bigint,
           -1.0::float8
    FROM disliked_tracks
),
canonical AS (
    SELECT user_id,
           CASE WHEN track_id ~ '^[0-9]{1,18}$' THEN track_id::bigint END AS track_id,
           event_code,
           unix_s,
           weight
    FROM events
    WHERE user_id ~ '^[0-9]{1,18}$'
)
SELECT user_id AS "user_id!",
       track_id AS "track_id!",
       event_code::smallint AS "event_code!",
       unix_s,
       weight AS "weight!"
FROM canonical
WHERE track_id > 0
ORDER BY user_id, unix_s NULLS FIRST, track_id, event_code
