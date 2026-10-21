SELECT id
FROM chat.reports
WHERE state = 'open'
  AND handled_by IS NULL
ORDER BY created_at
LIMIT $1
