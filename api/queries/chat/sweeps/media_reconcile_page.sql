SELECT object_key,
       coalesce(parent_id, id) AS "root!",
       object_key = ANY($2::text[]) AS "listed!"
FROM chat.media
WHERE store = $1
  AND (
      object_key = ANY($2::text[])
      OR (
          object_key > $3
          AND object_key <= $4
          AND status IN ('attached', 'uploaded')
      )
  )
