SELECT id
FROM chat.reports
WHERE state = 'open'
  AND handled_by IS NULL
  AND id <> ALL($2::uuid[])
ORDER BY created_at
LIMIT $1
