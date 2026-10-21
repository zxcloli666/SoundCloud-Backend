SELECT key_epoch,
       rekey,
       deleted_at IS NOT NULL AS "deleted!"
FROM chat.conversations
WHERE id = $1
FOR UPDATE
