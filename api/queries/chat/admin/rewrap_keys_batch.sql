SELECT conversation_id, epoch, wrapped, kek_version
FROM chat.conversation_keys
WHERE kek_version <> $1
LIMIT $2
