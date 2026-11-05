SELECT meta_ct,
       meta_epoch
FROM chat.conversations
WHERE id = $1
