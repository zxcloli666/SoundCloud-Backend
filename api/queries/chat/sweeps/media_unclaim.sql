UPDATE chat.media
SET lease_until = NULL,
    attempts = greatest(attempts - 1, 0)
WHERE (id = ANY($1::uuid[]) OR parent_id = ANY($1::uuid[]))
  AND status = 'deleting'
