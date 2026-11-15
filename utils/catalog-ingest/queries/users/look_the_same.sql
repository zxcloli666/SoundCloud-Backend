SELECT users.sc_user_id AS "sc_user_id!"
FROM jsonb_to_recordset($1::jsonb) AS seen(
    sc_user_id text, username text, avatar_url text, verified boolean, keys jsonb
)
JOIN users ON users.sc_user_id = seen.sc_user_id
WHERE users.username = seen.username
  AND NOT (seen.keys ? 'avatar_url' AND users.avatar_url IS DISTINCT FROM seen.avatar_url)
  AND NOT (seen.keys ? 'verified' AND users.verified IS DISTINCT FROM seen.verified)
