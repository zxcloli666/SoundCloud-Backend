SELECT node,
       id
FROM cmd_dead
WHERE ($1::text IS NULL OR node = $1)
  AND ($2::uuid IS NULL OR id = $2)
ORDER BY at, node, id
LIMIT $3
