DELETE FROM chat.media_refs
WHERE media_id = ANY($1::uuid[])
RETURNING media_id
