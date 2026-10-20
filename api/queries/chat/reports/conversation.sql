SELECT kind, room_ref, last_seq, (deleted_at IS NULL) AS "live!"
FROM chat.conversations
WHERE id = $1
