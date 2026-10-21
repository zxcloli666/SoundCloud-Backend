SELECT last_seq, key_epoch, member_count
FROM chat.conversations
WHERE id = $1
  AND kind = 'group'
  AND deleted_at IS NULL
FOR UPDATE
