SELECT id,
       kind,
       status
FROM chat.media
WHERE id = ANY($1::uuid[])
  AND kind NOT IN ('thumb', 'motion')
ORDER BY id
FOR NO KEY UPDATE
