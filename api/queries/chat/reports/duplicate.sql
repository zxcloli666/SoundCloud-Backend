SELECT id
FROM chat.reports
WHERE reporter = $1
  AND conversation_id = $2
  AND target_user = $3
  AND state = 'open'
  AND seq_from <= $5
  AND seq_to >= $4
LIMIT 1
