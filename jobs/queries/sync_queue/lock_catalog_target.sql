SELECT (
    (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM playlist_membership_state
            WHERE $1 IN ('playlist_update', 'playlist_delete', 'playlist_membership')
              AND playlist_urn = $2
            FOR UPDATE
        ) AS state
    ) + (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM playlists
            WHERE $1 IN ('playlist_update', 'playlist_delete')
              AND urn = $2
            FOR NO KEY UPDATE
        ) AS playlist
    ) + (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM tracks
            WHERE $1 IN ('track_update', 'track_delete')
              AND sc_track_id = regexp_replace($2, '^.*:', '')
            FOR NO KEY UPDATE
        ) AS track
    )
) AS "locked!"
