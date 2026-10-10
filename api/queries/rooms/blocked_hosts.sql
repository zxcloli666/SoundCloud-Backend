SELECT DISTINCT blocked AS "sc_user_id!"
FROM user_blocked_artists,
     unnest(sc_user_ids) AS blocked
WHERE sc_user_id = ANY ($1)
