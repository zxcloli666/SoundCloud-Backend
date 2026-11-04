UPDATE chat.media
SET status = 'attached',
    delete_after = NULL
WHERE (id = ANY($1::uuid[]) OR parent_id = ANY($1::uuid[]))
  AND status = 'uploaded'
