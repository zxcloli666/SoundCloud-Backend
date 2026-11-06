SELECT user_id,
       role,
       state,
       accepted,
       joined_seq,
       invited_by
FROM chat.members
WHERE conversation_id = $1
  AND role IN ('owner', 'admin')
  AND state = 'active'
ORDER BY role, user_id
