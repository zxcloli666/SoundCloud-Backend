SELECT $2::smallint + CASE
    WHEN EXISTS (
        SELECT 1
        FROM playlists AS playlist
        JOIN subscriptions AS subscription
          ON subscription.user_urn = playlist.owner_sc_user_id
        WHERE playlist.urn = $1
          AND subscription.exp_date > extract(epoch FROM now())::bigint
    ) THEN $3::smallint
    ELSE 0::smallint
END AS "priority!"
