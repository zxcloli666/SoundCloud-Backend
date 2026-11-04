SELECT id
FROM chat.conversations
WHERE deleted_at IS NOT NULL
  AND (created_by LIKE 'lt:%' OR dm_key LIKE '%lt:%')
ORDER BY deleted_at, id
LIMIT $1
