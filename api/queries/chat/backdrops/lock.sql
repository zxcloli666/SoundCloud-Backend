SELECT kind,
       settings,
       last_seq,
       (deleted_at IS NULL) AS "live!"
FROM chat.conversations
WHERE id = $1
FOR NO KEY UPDATE
