WITH star AS MATERIALIZED (
    SELECT subscription.user_urn AS user_id
    FROM subscriptions AS subscription
    WHERE subscription.exp_date > extract(epoch FROM now())::bigint
), star_ids AS MATERIALIZED (
    SELECT user_id AS id FROM star
    UNION ALL
    SELECT 'soundcloud:users:' || user_id FROM star
), liked AS MATERIALIZED (
    SELECT DISTINCT likes.sc_track_id
    FROM star_ids
    JOIN user_likes_tracks AS likes
      ON likes.user_id = star_ids.id
     AND likes.wanted_state
), other AS MATERIALIZED (
    SELECT member.sc_track_id
    FROM playlist_track_projection AS member
    WHERE member.playlist_urn = ANY (ARRAY(
        SELECT owned.playlist_urn
        FROM star_ids
        JOIN user_owned_playlists AS owned ON owned.user_id = star_ids.id
    ))
    UNION
    SELECT member.sc_track_id
    FROM star_ids
    JOIN user_likes_playlists AS liked_playlist
      ON liked_playlist.user_id = star_ids.id
     AND liked_playlist.wanted_state
    JOIN playlist_track_projection AS member ON member.playlist_urn = liked_playlist.playlist_urn
    UNION
    SELECT history.sc_track_id
    FROM star_ids
    JOIN listening_history AS history
      ON history.soundcloud_user_id = star_ids.id
     AND history.played_at > (now() AT TIME ZONE 'UTC') - $1::int * interval '1 day'
), wanted AS MATERIALIZED (
    SELECT liked.sc_track_id, 1::smallint AS tier
    FROM liked
    UNION ALL
    SELECT other.sc_track_id, 3::smallint
    FROM other
    WHERE NOT EXISTS (SELECT 1 FROM liked WHERE liked.sc_track_id = other.sc_track_id)
), removed AS (
    DELETE FROM star_track_tiers AS current
    WHERE NOT EXISTS (SELECT 1 FROM wanted WHERE wanted.sc_track_id = current.sc_track_id)
    RETURNING current.sc_track_id
), written AS (
    INSERT INTO star_track_tiers (sc_track_id, tier)
    SELECT wanted.sc_track_id, wanted.tier
    FROM wanted
    WHERE NOT EXISTS (
        SELECT 1
        FROM star_track_tiers AS current
        WHERE current.sc_track_id = wanted.sc_track_id
          AND current.tier = wanted.tier
    )
    ON CONFLICT (sc_track_id) DO UPDATE
    SET tier = EXCLUDED.tier,
        tiered_at = now()
    RETURNING star_track_tiers.sc_track_id
)
SELECT (SELECT count(*) FROM star) AS "star_users!",
       (SELECT count(*) FROM wanted WHERE tier = 1) AS "star_liked!",
       (SELECT count(*) FROM wanted WHERE tier = 3) AS "star_other!",
       (SELECT count(*) FROM written) AS "written!",
       (SELECT count(*) FROM removed) AS "removed!"
