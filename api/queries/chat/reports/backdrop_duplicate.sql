SELECT id,
       weight,
       state
FROM chat.reports
WHERE conversation_id = $1
  AND media_id = $2
  AND kind = 'backdrop'
  AND reporter = $3
ORDER BY created_at
LIMIT 1
