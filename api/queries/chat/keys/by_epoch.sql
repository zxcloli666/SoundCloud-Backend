SELECT epoch,
       wrapped,
       kek_version
FROM chat.conversation_keys
WHERE conversation_id = $1
  AND epoch = $2
