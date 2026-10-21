UPDATE chat.conversations
SET deleted_at = now()
WHERE deleted_at IS NULL
  AND (created_by LIKE 'lt:%' OR dm_key LIKE '%lt:%')
