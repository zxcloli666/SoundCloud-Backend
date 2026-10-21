SELECT created_at > localtimestamp - $2::int * interval '1 second' AS "young!"
FROM sessions
WHERE id = $1
