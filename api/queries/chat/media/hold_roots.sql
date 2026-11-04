SELECT DISTINCT media_id AS "media_id!"
FROM chat.media_refs
WHERE conversation_id = $1
  AND seq BETWEEN $2::bigint AND $3::bigint
  AND seq > 0
ORDER BY 1
