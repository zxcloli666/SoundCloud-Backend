UPDATE chat.media
SET delete_after = now() + $2::int * interval '1 second'
WHERE (id = $1 OR parent_id = $1)
  AND status = 'reserved'
RETURNING id
