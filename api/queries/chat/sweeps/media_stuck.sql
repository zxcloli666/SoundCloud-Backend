SELECT count(*) AS "stuck!"
FROM chat.media
WHERE status = 'deleting'
  AND kind NOT IN ('thumb', 'motion')
  AND attempts >= $1
