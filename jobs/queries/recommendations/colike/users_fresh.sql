CREATE TEMP TABLE colike_users AS
WITH ul AS (
    SELECT user_id,
           sc_track_id,
           row_number() OVER (
               PARTITION BY user_id
               ORDER BY created_at DESC, sc_track_id DESC
           ) AS rn
    FROM user_likes_tracks
    WHERE wanted_state = true
), ua AS (
    SELECT DISTINCT ul.user_id, track_artist.artist_id
    FROM ul
    JOIN tracks AS track ON track.sc_track_id = ul.sc_track_id
    JOIN track_artists AS track_artist
        ON track_artist.track_id = track.id
       AND track_artist.role = 'primary'
    JOIN artists AS artist
        ON artist.id = track_artist.artist_id
       AND artist.merged_into IS NULL
    WHERE ul.rn <= 500
)
SELECT user_id,
       artist_id,
       (hashtext(artist_id::text) & 2147483647) % $1::int AS shard
FROM ua
