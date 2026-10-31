WITH variants AS (
    SELECT DISTINCT variant.sc_user_id
    FROM unnest($2::text[]) AS variant(sc_user_id)
),
recent AS MATERIALIZED (
    SELECT variants.sc_user_id,
           kinds.event_type,
           latest.sc_track_id,
           latest.weight,
           latest.created_at
    FROM variants
    CROSS JOIN unnest(ARRAY['playlist_add', 'full_play', 'skip']::text[]) AS kinds(event_type)
    CROSS JOIN LATERAL (
        SELECT event.sc_track_id,
               event.weight,
               event.created_at
        FROM user_events AS event
        WHERE event.sc_user_id = variants.sc_user_id
          AND event.event_type = kinds.event_type
          AND event.created_at >= (now() AT TIME ZONE 'UTC') - $1::int * interval '1 day'
          AND (event.event_type <> 'skip' OR event.weight < 0)
        ORDER BY event.created_at DESC
        LIMIT $4
    ) AS latest
),
cutoffs AS (
    SELECT regexp_replace(truncated.sc_user_id, '^.*:', '') AS user_id,
           max(truncated.oldest_unix_s) AS cutoff_unix_s
    FROM (
        SELECT recent.sc_user_id,
               extract(epoch FROM min(recent.created_at))::bigint AS oldest_unix_s
        FROM recent
        GROUP BY recent.sc_user_id, recent.event_type
        HAVING count(*) >= $4
    ) AS truncated
    GROUP BY 1
),
window_events AS (
    SELECT regexp_replace(sc_user_id, '^.*:', '') AS user_id,
           regexp_replace(sc_track_id, '^.*:', '') AS track_id,
           event_type,
           weight,
           created_at
    FROM user_events
    WHERE sc_user_id = ANY ($2::text[])
      AND event_type = 'like'
      AND created_at >= (now() AT TIME ZONE 'UTC') - $1::int * interval '1 day'
    UNION ALL
    SELECT regexp_replace(sc_user_id, '^.*:', ''),
           regexp_replace(sc_track_id, '^.*:', ''),
           event_type,
           weight,
           created_at
    FROM recent
),
withdrawn_likes AS (
    SELECT regexp_replace(user_id, '^.*:', '') AS user_id,
           regexp_replace(sc_track_id, '^.*:', '') AS track_id
    FROM user_likes_tracks
    WHERE user_id = ANY ($2::text[])
      AND NOT wanted_state
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
    WHERE user_id = ANY ($2::text[])
      AND wanted_state
),
superliked AS MATERIALIZED (
    SELECT regexp_replace(s.user_id, '^.*:', '') AS user_id,
           regexp_replace(s.sc_track_id, '^.*:', '') AS track_id
    FROM user_superlikes s
    CROSS JOIN LATERAL (
        SELECT 1
        FROM user_likes_tracks l
        WHERE l.user_id = s.user_id
          AND l.sc_track_id = s.sc_track_id
          AND l.created_at = s.like_created_at
          AND l.wanted_state
        LIMIT 1
    ) live
    WHERE s.user_id = ANY ($2::text[])
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
    WHERE sc_user_id = ANY ($2::text[])
),
canonical AS (
    SELECT e.user_id,
           CASE WHEN e.track_id ~ '^[0-9]{1,18}$' THEN e.track_id::bigint END AS track_id,
           e.event_code,
           e.unix_s,
           CASE
               WHEN e.event_code IN (0, 1) AND EXISTS (
                   SELECT 1
                   FROM superliked sl
                   WHERE sl.user_id = e.user_id
                     AND sl.track_id = e.track_id
               ) THEN e.weight * $3::float8
               ELSE e.weight
           END AS weight
    FROM events e
    WHERE e.user_id ~ '^[0-9]{1,18}$'
)
SELECT canonical.user_id AS "user_id!",
       canonical.track_id AS "track_id!",
       canonical.event_code::smallint AS "event_code!",
       canonical.unix_s AS "unix_s?",
       canonical.weight AS "weight!",
       cutoffs.cutoff_unix_s AS "cutoff_unix_s?"
FROM canonical
LEFT JOIN cutoffs ON cutoffs.user_id = canonical.user_id
WHERE canonical.track_id > 0
ORDER BY canonical.user_id, canonical.unix_s NULLS FIRST, canonical.track_id, canonical.event_code
