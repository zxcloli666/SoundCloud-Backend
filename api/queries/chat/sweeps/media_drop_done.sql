DELETE FROM chat.media
WHERE status = 'deleting'
  AND (id = ANY($1::uuid[]) OR parent_id = ANY($1::uuid[]))
