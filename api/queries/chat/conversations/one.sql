SELECT c.id,
       c.kind,
       c.dm_key,
       c.room_ref,
       c.created_by,
       c.created_at,
       c.last_seq,
       c.last_at,
       c.key_epoch,
       c.meta_ct,
       c.meta_epoch,
       c.settings,
       c.member_count,
       c.ttl_s,
       m.role,
       m.state,
       m.accepted,
       m.joined_seq,
       m.joined_epoch,
       m.last_read_seq,
       m.muted_until,
       m.notify,
       m.pinned_at,
       m.archived,
       m.hidden_before_seq,
       m.invited_by
FROM chat.members m
JOIN chat.conversations c ON c.id = m.conversation_id
WHERE m.conversation_id = $1
  AND m.user_id = $2
  AND m.state IN ('active', 'invited')
  AND c.deleted_at IS NULL
