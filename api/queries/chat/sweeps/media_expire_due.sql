SELECT id
FROM chat.media
WHERE status IN ('reserved', 'uploaded')
  AND kind <> 'thumb'
  AND delete_after < now()
ORDER BY delete_after
LIMIT $1
FOR UPDATE SKIP LOCKED
