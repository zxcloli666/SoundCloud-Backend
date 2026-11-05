UPDATE chat.conversations
SET meta_ct = NULL,
    meta_epoch = NULL
WHERE id = $1
  AND kind = 'room'
