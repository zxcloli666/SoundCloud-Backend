SELECT user_id,
       role,
       state,
       accepted,
       joined_seq,
       invited_by
FROM chat.members
WHERE conversation_id = $1
  AND state IN ('active', 'invited', 'banned')
ORDER BY joined_seq, user_id
