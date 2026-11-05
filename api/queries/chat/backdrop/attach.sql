UPDATE chat.media
SET status = 'attached',
    delete_after = NULL
WHERE (id = $1 OR parent_id = $1)
  AND status = 'uploaded'
