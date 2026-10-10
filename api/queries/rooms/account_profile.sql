SELECT COALESCE(account.username, connection.username) AS username,
       account.avatar_url
FROM (SELECT $1::text AS sc_user_id) AS me
LEFT JOIN users AS account ON account.sc_user_id = me.sc_user_id
LEFT JOIN LATERAL (
    SELECT username
    FROM soundcloud_connections
    WHERE soundcloud_user_id = me.sc_user_id
      AND username IS NOT NULL
    ORDER BY updated_at DESC
    LIMIT 1
) AS connection ON true
