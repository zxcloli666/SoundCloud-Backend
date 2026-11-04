SELECT count(DISTINCT reporter) AS "reporters!"
FROM chat.reports
WHERE conversation_id = $1
  AND media_id = $2
  AND kind = 'backdrop'
  AND weight > 0
  AND created_at > now() - $3::int * interval '1 second'
