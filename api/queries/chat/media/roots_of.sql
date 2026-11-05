SELECT id,
       parent_id AS "root!"
FROM chat.media
WHERE id = ANY($1::uuid[])
  AND owner_user = $2
  AND parent_id IS NOT NULL
