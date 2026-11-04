SELECT id,
       kind,
       room_ref,
       created_by,
       created_at,
       last_seq,
       last_at,
       key_epoch,
       meta_ct,
       meta_epoch,
       settings,
       member_count,
       ttl_s
FROM chat.conversations
WHERE id = $1
  AND kind = 'room'
  AND deleted_at IS NULL
