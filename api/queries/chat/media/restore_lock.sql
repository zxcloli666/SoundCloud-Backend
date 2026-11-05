SELECT media.id
FROM chat.media AS media
WHERE media.owner_user = $1
  AND media.kind <> 'thumb'
  AND media.status = 'deleting'
  AND media.restorable_until > now()
  AND media.lease_until IS NULL
  AND ($3::boolean OR media.id IN (
      SELECT coalesce(picked.parent_id, picked.id)
      FROM chat.media AS picked
      WHERE picked.id = ANY($2::uuid[])
        AND picked.owner_user = $1
  ))
ORDER BY media.id
FOR UPDATE
