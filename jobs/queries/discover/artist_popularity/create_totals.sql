CREATE TEMP TABLE artist_play_totals AS
WITH track_full_plays AS (
    SELECT sc_track_id,
           count(*)::bigint * $1::bigint AS plays
    FROM user_events
    WHERE event_type = 'full_play'
    GROUP BY sc_track_id
), track_plays AS (
    SELECT sc_track_id, COALESCE(play_count, 0) AS plays
    FROM sc_track_counters
    UNION ALL
    SELECT sc_track_id, plays
    FROM track_full_plays
)
SELECT track.primary_artist_id AS artist_id,
       sum(track_plays.plays)::bigint AS score
FROM track_plays
JOIN tracks AS track ON track.sc_track_id = track_plays.sc_track_id
WHERE track.primary_artist_id IS NOT NULL
GROUP BY track.primary_artist_id
