SELECT count(*) AS "stuck!"
FROM chat.media
WHERE status = 'deleting'
  AND kind <> 'thumb'
  AND attempts >= $1
