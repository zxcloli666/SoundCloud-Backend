SELECT id,
       kind,
       status
FROM chat.media
WHERE id = ANY($1::uuid[])
  AND kind <> 'thumb'
ORDER BY id
FOR NO KEY UPDATE
