SELECT wanted AS "sc_user_id!"
FROM unnest($1::text[]) AS wanted
WHERE NOT EXISTS (SELECT 1 FROM users WHERE users.sc_user_id = wanted)
