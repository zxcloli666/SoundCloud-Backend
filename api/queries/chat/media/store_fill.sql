SELECT coalesce(sum(bytes), 0)::bigint AS "bytes!"
FROM chat.media
WHERE store = $1
  AND status <> 'lost'
